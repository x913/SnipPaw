use keyring::{Entry, Error as KeyringError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &["mp4"];
const SUPPORTED_SUBTITLE_EXTENSIONS: &[&str] = &["srt", "vtt"];
const YOUTUBE_MP4_FORMAT: &str = "bv*[vcodec^=avc1][height<=1080]+ba[acodec^=mp4a]/b[ext=mp4]";
const OPENAI_MODEL: &str = "gpt-4.1-mini";
const OPENAI_KEYRING_SERVICE: &str = "com.snippaw.desktop";
const LEGACY_OPENAI_KEYRING_SERVICE: &str = "com.videocut.desktop";
const OPENAI_KEYRING_ACCOUNT: &str = "openai-api-key";
const LEGACY_APP_IDENTIFIER: &str = "com.videocut.desktop";
const WAVEFORM_SAMPLE_RATE: u32 = 2_000;
const WAVEFORM_PEAKS_PER_SECOND: u32 = 4;
const WAVEFORM_SAMPLES_PER_PEAK: usize =
    (WAVEFORM_SAMPLE_RATE / WAVEFORM_PEAKS_PER_SECOND) as usize;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VideoFile {
    path: String,
    file_name: String,
    file_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubtitleCue {
    start_seconds: f64,
    end_seconds: f64,
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AiMoment {
    start_cue_index: usize,
    end_cue_index: usize,
    start_seconds: f64,
    end_seconds: f64,
    note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AiAnalysis {
    instruction: String,
    model: String,
    created_at_unix_ms: u128,
    subtitle_fingerprint: String,
    moments: Vec<AiMoment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Project {
    id: String,
    name: String,
    video: VideoFile,
    #[serde(default)]
    subtitles: Vec<SubtitleCue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ai_analysis: Option<AiAnalysis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_path: Option<String>,
    created_at_unix_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectDraft {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Serialize)]
struct ImportToolsStatus {
    yt_dlp_available: bool,
    ffmpeg_available: bool,
    deno_available: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AiKeyStatus {
    Saved,
    Unknown,
    NotConfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AiKeyMarker {
    api_key_configured: bool,
}

#[derive(Default)]
struct OpenAiKeyCache(Mutex<Option<String>>);

#[derive(Debug, Clone, Serialize)]
struct AiSettings {
    key_status: AiKeyStatus,
    model: &'static str,
}

#[derive(Debug, Deserialize)]
struct OpenAiAnalysisResponse {
    moments: Vec<OpenAiMoment>,
}

#[derive(Debug, Deserialize)]
struct OpenAiMoment {
    source_cue_index: usize,
    note: String,
}

#[derive(Debug, Clone, Serialize)]
struct YoutubeVideoInfo {
    title: String,
}

#[derive(Debug, Clone, Serialize)]
struct YoutubeDownloadProgress {
    draft_id: String,
    phase: &'static str,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    speed_bytes_per_second: Option<f64>,
    eta_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct ClipExport {
    output_path: String,
    file_name: String,
    duration_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AudioWaveform {
    source_path: String,
    source_file_size_bytes: u64,
    source_modified_unix_ms: Option<u128>,
    peaks_per_second: u32,
    peaks: Vec<u16>,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectDeletion {
    source_video_deleted: bool,
    exported_clips_deleted: usize,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExportQuality {
    High,
    Balanced,
    Compact,
}

impl ExportQuality {
    fn ffmpeg_settings(self) -> (&'static str, &'static str) {
        match self {
            Self::High => ("20", "192k"),
            Self::Balanced => ("23", "160k"),
            Self::Compact => ("28", "128k"),
        }
    }
}

struct PreviewServer {
    address: String,
    sources: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl PreviewServer {
    fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let sources = Arc::new(Mutex::new(HashMap::<String, PathBuf>::new()));
        let server_sources = Arc::clone(&sources);

        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let request_sources = Arc::clone(&server_sources);
                thread::spawn(move || {
                    let _ = serve_preview_request(stream, request_sources);
                });
            }
        });

        Ok(Self { address, sources })
    }

    fn register(&self, path: PathBuf) -> Result<String, String> {
        let token = Uuid::new_v4().to_string();
        self.sources
            .lock()
            .map_err(|_| "Не удалось подготовить предпросмотр видео.".to_string())?
            .insert(token.clone(), path);
        Ok(format!("http://{}/preview/{token}", self.address))
    }
}

#[tauri::command]
fn inspect_video(path: String) -> Result<VideoFile, String> {
    inspect_video_file(Path::new(&path))
}

#[tauri::command]
fn preview_video_url(app: AppHandle, project_id: String) -> Result<String, String> {
    let project = load_project(&app, &project_id)?;
    let source_path = PathBuf::from(project.video.path);
    if !source_path.is_file() {
        return Err("Исходное видео не найдено. Укажите его заново в проекте.".to_string());
    }

    app.state::<PreviewServer>().register(source_path)
}

#[tauri::command]
async fn audio_waveform(app: AppHandle, project_id: String) -> Result<AudioWaveform, String> {
    let waveform_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        load_or_create_audio_waveform(&waveform_app, &project_id)
    })
    .await
    .map_err(|error| format!("Не удалось подготовить аудиодорожку: {error}"))?
}

#[tauri::command]
fn list_projects(app: AppHandle) -> Result<Vec<Project>, String> {
    let root = projects_root(&app)?;
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut projects = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|error| format!("Не удалось прочитать проекты: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("Не удалось прочитать проект: {error}"))?
            .path()
            .join("project.json");
        if !path.is_file() {
            continue;
        }

        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("Не удалось прочитать {}: {error}", path.display()))?;
        let project = serde_json::from_str::<Project>(&contents)
            .map_err(|error| format!("Не удалось прочитать {}: {error}", path.display()))?;
        projects.push(project);
    }

    projects.sort_by(|left, right| right.created_at_unix_ms.cmp(&left.created_at_unix_ms));
    Ok(projects)
}

#[tauri::command]
fn import_subtitles(app: AppHandle, project_id: String, path: String) -> Result<Project, String> {
    let subtitles = parse_subtitle_file(Path::new(&path))?;
    let mut project = load_project(&app, &project_id)?;
    project.subtitles = subtitles;
    project.ai_analysis = None;
    save_project(&app, &project)?;
    Ok(project)
}

#[tauri::command]
fn get_ai_settings(app: AppHandle) -> Result<AiSettings, String> {
    ai_settings_from_marker(&app)
}

#[tauri::command]
fn save_openai_api_key(
    app: AppHandle,
    key_cache: tauri::State<OpenAiKeyCache>,
    api_key: String,
) -> Result<AiSettings, String> {
    let api_key = normalized_openai_api_key(&api_key)?;
    openai_keyring_entry()?
        .set_password(&api_key)
        .map_err(|error| format!("Не удалось сохранить ключ в macOS Keychain: {error}"))?;
    cache_openai_api_key(key_cache.inner(), api_key)?;
    save_ai_key_marker(&app, true)?;
    ai_settings_from_marker(&app)
}

#[tauri::command]
fn delete_openai_api_key(
    app: AppHandle,
    key_cache: tauri::State<OpenAiKeyCache>,
) -> Result<AiSettings, String> {
    delete_keyring_entry(openai_keyring_entry()?)?;
    delete_keyring_entry(legacy_openai_keyring_entry()?)?;
    clear_openai_api_key(key_cache.inner())?;
    save_ai_key_marker(&app, false)?;
    ai_settings_from_marker(&app)
}

#[tauri::command]
async fn analyze_subtitles(
    app: AppHandle,
    key_cache: tauri::State<'_, OpenAiKeyCache>,
    project_id: String,
    instruction: String,
) -> Result<Project, String> {
    let instruction = normalized_ai_instruction(&instruction)?;
    let api_key = stored_openai_api_key(&app, key_cache.inner())?;
    let mut project = load_project(&app, &project_id)?;
    if project.subtitles.is_empty() {
        return Err("Сначала загрузите субтитры SRT или VTT.".to_string());
    }

    let response =
        request_openai_subtitle_analysis(&api_key, &instruction, &project.subtitles).await?;
    let moments = validate_ai_moments(response.moments, &project.subtitles)?;
    project.ai_analysis = Some(AiAnalysis {
        instruction,
        model: OPENAI_MODEL.to_string(),
        created_at_unix_ms: now_unix_ms()?,
        subtitle_fingerprint: subtitle_fingerprint(&project.subtitles),
        moments,
    });
    save_project(&app, &project)?;
    Ok(project)
}

#[tauri::command]
fn delete_project(
    app: AppHandle,
    project_id: String,
    delete_source_video: bool,
    delete_exported_clips: bool,
) -> Result<ProjectDeletion, String> {
    let project = load_project(&app, &project_id)?;
    let project_directory = project_directory(&app, &project_id)?;
    let exported_clips_deleted = if delete_exported_clips {
        remove_exported_clips(&app, &project)?
    } else {
        0
    };
    let source_video_deleted = if delete_source_video {
        let source_path = PathBuf::from(&project.video.path);
        if source_path.is_file() {
            fs::remove_file(&source_path)
                .map_err(|error| format!("Не удалось удалить исходное видео: {error}"))?;
            true
        } else {
            false
        }
    } else {
        false
    };

    fs::remove_dir_all(&project_directory)
        .map_err(|error| format!("Не удалось удалить проект: {error}"))?;
    Ok(ProjectDeletion {
        source_video_deleted,
        exported_clips_deleted,
    })
}

#[tauri::command]
fn open_exports_folder(app: AppHandle, project_id: String) -> Result<(), String> {
    let project = load_project(&app, &project_id)?;
    let directory = exports_directory_path(&app, &project)?;
    if !directory.is_dir() {
        return Err("Вырезанные клипы для этого проекта ещё не создавались.".to_string());
    }

    let status = Command::new("open")
        .arg(&directory)
        .status()
        .map_err(|error| format!("Не удалось открыть Finder: {error}"))?;
    if !status.success() {
        return Err("Не удалось открыть папку с клипами в Finder.".to_string());
    }
    Ok(())
}

#[tauri::command]
fn check_import_tools() -> ImportToolsStatus {
    let yt_dlp_available = find_yt_dlp()
        .ok()
        .is_some_and(|path| command_is_available(&path, "--version"));
    let ffmpeg_available = find_ffmpeg()
        .ok()
        .is_some_and(|path| command_is_available(&path, "-version"));
    let deno_available = find_deno()
        .ok()
        .is_some_and(|path| command_is_available(&path, "--version"));

    ImportToolsStatus {
        yt_dlp_available,
        ffmpeg_available,
        deno_available,
    }
}

#[tauri::command]
fn prepare_project_import(app: AppHandle, name: String) -> Result<ProjectDraft, String> {
    let name = normalized_project_name(&name)?;
    let draft = ProjectDraft {
        id: Uuid::new_v4().to_string(),
        name,
    };
    let project_dir = project_directory(&app, &draft.id)?;
    fs::create_dir_all(&project_dir)
        .map_err(|error| format!("Не удалось создать папку проекта: {error}"))?;

    let workspace = project_workspace_directory(&app, &draft)?;
    fs::create_dir_all(workspace.join("source"))
        .map_err(|error| format!("Не удалось создать папку для файлов проекта: {error}"))?;
    save_project_draft(&app, &draft)?;

    Ok(draft)
}

#[tauri::command]
fn discard_project_draft(app: AppHandle, draft_id: String) -> Result<(), String> {
    let draft = load_project_draft(&app, &draft_id)?;
    let project_dir = project_directory(&app, &draft.id)?;
    let workspace = project_workspace_directory(&app, &draft)?;

    fs::remove_dir_all(&project_dir)
        .map_err(|error| format!("Не удалось удалить черновик проекта: {error}"))?;
    if workspace.is_dir() {
        fs::remove_dir_all(&workspace)
            .map_err(|error| format!("Не удалось удалить файлы черновика: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
fn complete_file_import(
    app: AppHandle,
    draft_id: String,
    video_path: String,
    copy_source_video: bool,
) -> Result<Project, String> {
    let draft = load_project_draft(&app, &draft_id)?;
    let source_path = PathBuf::from(video_path);
    let video = if copy_source_video {
        copy_video_to_project_workspace(&app, &draft, &source_path)?
    } else {
        inspect_video_file(&source_path)?
    };
    finalize_project_draft(&app, draft, video)
}

#[tauri::command]
async fn download_youtube_video(
    app: AppHandle,
    draft_id: String,
    url: String,
    use_chrome_cookies: bool,
) -> Result<Project, String> {
    let draft = load_project_draft(&app, &draft_id)?;
    validate_youtube_url(&url)?;
    let yt_dlp = find_yt_dlp()?;
    let ffmpeg = find_ffmpeg()?;
    let deno = find_deno()?;
    if !command_is_available(&yt_dlp, "--version") {
        return Err(
            "yt-dlp не найден. Установите yt-dlp, чтобы скачать видео по ссылке.".to_string(),
        );
    }
    if !command_is_available(&ffmpeg, "-version") {
        return Err(
            "FFmpeg не найден. Установите FFmpeg, чтобы скачать видео по ссылке.".to_string(),
        );
    }
    if !command_is_available(&deno, "--version") {
        return Err(
            "Deno не найден. Установите Deno, чтобы скачать видео по ссылке YouTube.".to_string(),
        );
    }

    let workspace = project_workspace_directory(&app, &draft)?;
    let source_directory = workspace.join("source");
    let progress_app = app.clone();
    let progress_draft_id = draft.id.clone();
    let downloaded_path = tauri::async_runtime::spawn_blocking(move || {
        download_youtube_video_file(
            &progress_app,
            &progress_draft_id,
            &yt_dlp,
            &ffmpeg,
            &deno,
            &source_directory,
            &url,
            use_chrome_cookies,
        )
    })
    .await
    .map_err(|error| format!("Не удалось дождаться загрузки видео: {error}"))??;
    let video = inspect_video_file(&downloaded_path)?;
    finalize_project_draft(&app, draft, video)
}

#[tauri::command]
async fn resolve_youtube_video_title(
    url: String,
    use_chrome_cookies: bool,
) -> Result<YoutubeVideoInfo, String> {
    validate_youtube_url(&url)?;
    let yt_dlp = find_yt_dlp()?;
    let deno = find_deno()?;
    if !command_is_available(&yt_dlp, "--version") {
        return Err(
            "yt-dlp не найден. Установите yt-dlp, чтобы получить название видео.".to_string(),
        );
    }
    if !command_is_available(&deno, "--version") {
        return Err(
            "Deno не найден. Установите Deno, чтобы получить название видео YouTube.".to_string(),
        );
    }

    let title = tauri::async_runtime::spawn_blocking(move || {
        fetch_youtube_video_title(&yt_dlp, &deno, &url, use_chrome_cookies)
    })
    .await
    .map_err(|error| format!("Не удалось дождаться yt-dlp: {error}"))??;
    Ok(YoutubeVideoInfo { title })
}

#[tauri::command]
async fn export_clip(
    app: AppHandle,
    project_id: String,
    start_seconds: f64,
    end_seconds: f64,
    file_name_hint: Option<String>,
    quality: ExportQuality,
) -> Result<ClipExport, String> {
    validate_clip_bounds(start_seconds, end_seconds)?;
    let project = load_project(&app, &project_id)?;
    let source_path = PathBuf::from(&project.video.path);
    if !source_path.is_file() {
        return Err("Исходное видео не найдено. Укажите его заново в проекте.".to_string());
    }

    let output_directory = exports_directory(&app, &project)?;
    let output_path = next_export_path(
        &output_directory,
        file_name_hint.as_deref(),
        start_seconds,
        end_seconds,
    );
    let output_file_name = output_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Не удалось подготовить имя выходного файла.".to_string())?
        .to_owned();
    let duration_seconds = end_seconds - start_seconds;

    tauri::async_runtime::spawn_blocking(move || {
        transcode_clip(
            &source_path,
            &output_path,
            start_seconds,
            duration_seconds,
            quality,
        )?;
        Ok(ClipExport {
            output_path: output_path.to_string_lossy().into_owned(),
            file_name: output_file_name,
            duration_seconds,
        })
    })
    .await
    .map_err(|error| format!("Экспорт был прерван: {error}"))?
}

fn serve_preview_request(
    mut stream: TcpStream,
    sources: Arc<Mutex<HashMap<String, PathBuf>>>,
) -> io::Result<()> {
    let request = read_http_request(&mut stream)?;
    let mut lines = request.lines();
    let Some(request_line) = lines.next() else {
        return Ok(());
    };
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default();
    let request_path = request_parts.next().unwrap_or_default();
    let token = request_path
        .strip_prefix("/preview/")
        .and_then(|path| path.split('?').next())
        .filter(|token| !token.is_empty());
    let Some(token) = token else {
        return write_http_error(&mut stream, "404 Not Found");
    };

    let source_path = sources
        .lock()
        .ok()
        .and_then(|paths| paths.get(token).cloned());
    let Some(source_path) = source_path else {
        return write_http_error(&mut stream, "404 Not Found");
    };
    if method != "GET" && method != "HEAD" {
        return write_http_error(&mut stream, "405 Method Not Allowed");
    }

    let mut file = fs::File::open(&source_path)?;
    let file_length = file.metadata()?.len();
    let range_header = request
        .lines()
        .find_map(|line| {
            line.strip_prefix("Range:")
                .or_else(|| line.strip_prefix("range:"))
        })
        .map(str::trim);
    let byte_range = match range_header {
        Some(header) => parse_byte_range(header, file_length),
        None if file_length > 0 => Some((0, file_length - 1)),
        None => None,
    };
    let Some((start, end)) = byte_range else {
        return write_range_not_satisfiable(&mut stream, file_length);
    };

    let content_length = end - start + 1;
    let status = if range_header.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    let content_type = preview_content_type(&source_path);
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {content_length}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n"
    )?;
    if range_header.is_some() {
        write!(
            stream,
            "Content-Range: bytes {start}-{end}/{file_length}\r\n"
        )?;
    }
    stream.write_all(b"\r\n")?;

    if method == "HEAD" {
        return Ok(());
    }

    file.seek(SeekFrom::Start(start))?;
    let mut remaining = content_length;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let chunk_length = remaining.min(buffer.len() as u64) as usize;
        let bytes_read = file.read(&mut buffer[..chunk_length])?;
        if bytes_read == 0 {
            break;
        }
        stream.write_all(&buffer[..bytes_read])?;
        remaining -= bytes_read as u64;
    }
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<String> {
    const MAX_REQUEST_HEADER_SIZE: usize = 16 * 1024;
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];

    while request.len() < MAX_REQUEST_HEADER_SIZE {
        let bytes_read = stream.read(&mut chunk)?;
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..bytes_read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    Ok(String::from_utf8_lossy(&request).into_owned())
}

fn parse_byte_range(header: &str, file_length: u64) -> Option<(u64, u64)> {
    let value = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (start, end) = value.split_once('-')?;
    if file_length == 0 {
        return None;
    }

    if start.is_empty() {
        let suffix_length = end.parse::<u64>().ok()?.min(file_length);
        if suffix_length == 0 {
            return None;
        }
        return Some((file_length - suffix_length, file_length - 1));
    }

    let start = start.parse::<u64>().ok()?;
    if start >= file_length {
        return None;
    }
    let end = if end.is_empty() {
        file_length - 1
    } else {
        end.parse::<u64>().ok()?.min(file_length - 1)
    };
    (end >= start).then_some((start, end))
}

fn preview_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "m4v" => "video/x-m4v",
        "avi" => "video/x-msvideo",
        _ => "video/mp4",
    }
}

fn write_http_error(stream: &mut TcpStream, status: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn write_range_not_satisfiable(stream: &mut TcpStream, file_length: u64) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{file_length}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn load_project(app: &AppHandle, project_id: &str) -> Result<Project, String> {
    let path = project_directory(app, project_id)?.join("project.json");
    let contents =
        fs::read_to_string(&path).map_err(|_| "Не удалось найти указанный проект.".to_string())?;
    serde_json::from_str(&contents).map_err(|error| format!("Не удалось прочитать проект: {error}"))
}

fn save_project(app: &AppHandle, project: &Project) -> Result<(), String> {
    let path = project_directory(app, &project.id)?.join("project.json");
    let contents = serde_json::to_string_pretty(project)
        .map_err(|error| format!("Не удалось сохранить проект: {error}"))?;
    fs::write(path, contents).map_err(|error| format!("Не удалось сохранить проект: {error}"))
}

fn save_project_draft(app: &AppHandle, draft: &ProjectDraft) -> Result<(), String> {
    let path = project_directory(app, &draft.id)?.join("draft.json");
    let contents = serde_json::to_string_pretty(draft)
        .map_err(|error| format!("Не удалось сохранить черновик проекта: {error}"))?;
    fs::write(path, contents)
        .map_err(|error| format!("Не удалось сохранить черновик проекта: {error}"))
}

fn load_project_draft(app: &AppHandle, draft_id: &str) -> Result<ProjectDraft, String> {
    let path = project_directory(app, draft_id)?.join("draft.json");
    let contents =
        fs::read_to_string(&path).map_err(|_| "Не удалось найти черновик проекта.".to_string())?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("Не удалось прочитать черновик проекта: {error}"))
}

fn finalize_project_draft(
    app: &AppHandle,
    draft: ProjectDraft,
    video: VideoFile,
) -> Result<Project, String> {
    let workspace_path = project_workspace_directory(app, &draft)?;
    let draft_id = draft.id.clone();
    let project = Project {
        id: draft_id.clone(),
        name: draft.name,
        video,
        subtitles: Vec::new(),
        ai_analysis: None,
        workspace_path: Some(workspace_path.to_string_lossy().into_owned()),
        created_at_unix_ms: now_unix_ms()?,
    };
    save_project(app, &project)?;

    let draft_path = project_directory(app, &draft_id)?.join("draft.json");
    fs::remove_file(draft_path)
        .map_err(|error| format!("Не удалось завершить создание проекта: {error}"))?;
    Ok(project)
}

fn copy_video_to_project_workspace(
    app: &AppHandle,
    draft: &ProjectDraft,
    source_path: &Path,
) -> Result<VideoFile, String> {
    let source_video = inspect_video_file(source_path)?;
    let source_directory = project_workspace_directory(app, draft)?.join("source");
    fs::create_dir_all(&source_directory)
        .map_err(|error| format!("Не удалось создать папку исходного видео: {error}"))?;
    let destination = source_directory.join(&source_video.file_name);
    if destination.exists() {
        return Err("В папке проекта уже есть файл с таким именем.".to_string());
    }

    fs::copy(source_path, &destination)
        .map_err(|error| format!("Не удалось скопировать исходное видео: {error}"))?;
    inspect_video_file(&destination)
}

fn parse_subtitle_file(path: &Path) -> Result<Vec<SubtitleCue>, String> {
    if !path.is_file() {
        return Err("Файл субтитров не найден.".to_string());
    }
    if !is_supported_subtitle(path) {
        return Err("Поддерживаются субтитры SRT и VTT.".to_string());
    }

    let contents = fs::read_to_string(path).map_err(|_| {
        "Не удалось прочитать субтитры. Используйте файл в кодировке UTF-8.".to_string()
    })?;
    parse_subtitle_cues(&contents)
}

fn parse_subtitle_cues(contents: &str) -> Result<Vec<SubtitleCue>, String> {
    let normalized = contents
        .strip_prefix('\u{feff}')
        .unwrap_or(contents)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        let lines = block
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let Some(timeline_index) = lines.iter().position(|line| line.contains("-->")) else {
            continue;
        };
        let (start_seconds, end_seconds) = parse_subtitle_range(lines[timeline_index])?;
        let text = lines[timeline_index + 1..]
            .iter()
            .map(|line| strip_subtitle_markup(line))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        if !text.is_empty() {
            cues.push(SubtitleCue {
                start_seconds,
                end_seconds,
                text,
            });
        }
    }

    if cues.is_empty() {
        return Err("В файле не найдены реплики с таймкодами.".to_string());
    }
    Ok(cues)
}

fn parse_subtitle_range(value: &str) -> Result<(f64, f64), String> {
    let (start, end_with_settings) = value
        .split_once("-->")
        .ok_or_else(|| "Не удалось прочитать интервал субтитров.".to_string())?;
    let end = end_with_settings
        .split_whitespace()
        .next()
        .ok_or_else(|| "Не удалось прочитать конец интервала субтитров.".to_string())?;
    let start_seconds = parse_subtitle_time(start)?;
    let end_seconds = parse_subtitle_time(end)?;
    if end_seconds <= start_seconds {
        return Err("Конец реплики должен быть позже её начала.".to_string());
    }
    Ok((start_seconds, end_seconds))
}

fn parse_subtitle_time(value: &str) -> Result<f64, String> {
    let normalized = value.trim().replace(',', ".");
    let parts = normalized.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return Err("Не удалось прочитать таймкод субтитров.".to_string());
    }

    let seconds = parts[parts.len() - 1]
        .parse::<f64>()
        .map_err(|_| "Не удалось прочитать таймкод субтитров.".to_string())?;
    let minutes = parts[parts.len() - 2]
        .parse::<u64>()
        .map_err(|_| "Не удалось прочитать таймкод субтитров.".to_string())?;
    let hours = if parts.len() == 3 {
        parts[0]
            .parse::<u64>()
            .map_err(|_| "Не удалось прочитать таймкод субтитров.".to_string())?
    } else {
        0
    };
    if !seconds.is_finite() || seconds < 0.0 || seconds >= 60.0 || minutes >= 60 {
        return Err("Не удалось прочитать таймкод субтитров.".to_string());
    }
    Ok(hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds)
}

fn strip_subtitle_markup(value: &str) -> String {
    let mut result = String::new();
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => result.push(character),
            _ => {}
        }
    }
    result.trim().to_string()
}

fn validate_clip_bounds(start_seconds: f64, end_seconds: f64) -> Result<(), String> {
    let duration_seconds = end_seconds - start_seconds;
    if !start_seconds.is_finite()
        || !end_seconds.is_finite()
        || start_seconds < 0.0
        || duration_seconds < 0.1 - 0.000_001
    {
        return Err("Укажите корректные границы клипа длиной не менее 0,1 секунды.".to_string());
    }
    Ok(())
}

fn exports_directory(app: &AppHandle, project: &Project) -> Result<PathBuf, String> {
    let directory = exports_directory_path(app, project)?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Не удалось создать папку для клипов: {error}"))?;
    Ok(directory)
}

fn exports_directory_path(app: &AppHandle, project: &Project) -> Result<PathBuf, String> {
    if let Some(workspace_path) = &project.workspace_path {
        return Ok(PathBuf::from(workspace_path).join("clips"));
    }

    let base = app
        .path()
        .video_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|error| format!("Не удалось определить папку экспорта: {error}"))?;
    Ok(base
        .join("VideoCut")
        .join(safe_directory_name(&project.name)))
}

fn remove_exported_clips(app: &AppHandle, project: &Project) -> Result<usize, String> {
    let directory = exports_directory_path(app, project)?;
    if !directory.is_dir() {
        return Ok(0);
    }

    if project.workspace_path.is_some() {
        return delete_mp4_clips_in_directory(&directory);
    }

    let projects_with_same_export_folder = list_projects(app.clone())?
        .into_iter()
        .filter(|candidate| {
            candidate.id != project.id
                && safe_directory_name(&candidate.name) == safe_directory_name(&project.name)
        })
        .count();
    if projects_with_same_export_folder > 0 {
        return Err("Нельзя безопасно удалить клипы: другой проект использует ту же папку экспорта. Удалите клипы вручную из этой папки.".to_string());
    }

    delete_mp4_clips_in_directory(&directory)
}

fn delete_mp4_clips_in_directory(directory: &Path) -> Result<usize, String> {
    let mut deleted = 0;
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Не удалось прочитать папку клипов: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("Не удалось прочитать клип: {error}"))?
            .path();
        let is_mp4 = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
        if path.is_file() && is_mp4 {
            fs::remove_file(&path)
                .map_err(|error| format!("Не удалось удалить клип {}: {error}", path.display()))?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

fn next_export_path(
    directory: &Path,
    file_name_hint: Option<&str>,
    start_seconds: f64,
    end_seconds: f64,
) -> PathBuf {
    let fallback_stem = format!(
        "clip_{}_{}",
        filename_timecode(start_seconds),
        filename_timecode(end_seconds)
    );
    let stem = file_name_hint
        .map(safe_clip_filename_stem)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_stem);
    let mut candidate = directory.join(format!("{stem}.mp4"));
    let mut suffix = 2;
    while candidate.exists() {
        candidate = directory.join(format!("{stem}_{suffix}.mp4"));
        suffix += 1;
    }
    candidate
}

fn transcode_clip(
    source_path: &Path,
    output_path: &Path,
    start_seconds: f64,
    duration_seconds: f64,
    quality: ExportQuality,
) -> Result<(), String> {
    let ffmpeg = find_ffmpeg()?;
    let (crf, audio_bitrate) = quality.ffmpeg_settings();
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss"])
        .arg(format_seconds(start_seconds))
        .arg("-i")
        .arg(source_path)
        .arg("-t")
        .arg(format_seconds(duration_seconds))
        .args([
            "-map",
            "0:v:0?",
            "-map",
            "0:a?",
            "-c:v",
            "libx264",
            "-crf",
            crf,
            "-preset",
            "medium",
            "-c:a",
            "aac",
            "-b:a",
            audio_bitrate,
            "-movflags",
            "+faststart",
        ])
        .arg(output_path)
        .output()
        .map_err(|error| format!("Не удалось запустить FFmpeg: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let details = String::from_utf8_lossy(&output.stderr);
    let details = details.trim();
    let details = if details.is_empty() {
        "FFmpeg завершился с ошибкой.".to_string()
    } else {
        details.chars().take(600).collect()
    };
    Err(format!("Не удалось вырезать видео: {details}"))
}

fn load_or_create_audio_waveform(
    app: &AppHandle,
    project_id: &str,
) -> Result<AudioWaveform, String> {
    let project = load_project(app, project_id)?;
    let source_path = PathBuf::from(&project.video.path);
    let source_metadata = fs::metadata(&source_path)
        .map_err(|_| "Исходное видео не найдено. Укажите его заново в проекте.".to_string())?;
    let source_modified_unix_ms = source_modified_unix_ms(&source_metadata);
    let cache_path = project_directory(app, project_id)?.join("audio-waveform.json");

    if let Ok(contents) = fs::read_to_string(&cache_path) {
        if let Ok(cached) = serde_json::from_str::<AudioWaveform>(&contents) {
            if cached.source_path == project.video.path
                && cached.source_file_size_bytes == source_metadata.len()
                && cached.source_modified_unix_ms == source_modified_unix_ms
                && cached.peaks_per_second == WAVEFORM_PEAKS_PER_SECOND
            {
                return Ok(cached);
            }
        }
    }

    let waveform = build_audio_waveform(
        &source_path,
        project.video.path,
        source_metadata.len(),
        source_modified_unix_ms,
    )?;
    let contents = serde_json::to_string(&waveform)
        .map_err(|error| format!("Не удалось сохранить аудиодорожку: {error}"))?;
    fs::write(cache_path, contents)
        .map_err(|error| format!("Не удалось сохранить аудиодорожку: {error}"))?;
    Ok(waveform)
}

fn source_modified_unix_ms(metadata: &fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
}

fn build_audio_waveform(
    source_path: &Path,
    source_path_string: String,
    source_file_size_bytes: u64,
    source_modified_unix_ms: Option<u128>,
) -> Result<AudioWaveform, String> {
    let ffmpeg = find_ffmpeg()?;
    let sample_rate = WAVEFORM_SAMPLE_RATE.to_string();
    let mut process = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(source_path)
        .args([
            "-map",
            "0:a:0?",
            "-vn",
            "-ac",
            "1",
            "-ar",
            sample_rate.as_str(),
            "-f",
            "s16le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Не удалось запустить FFmpeg: {error}"))?;
    let stdout = process
        .stdout
        .take()
        .ok_or_else(|| "Не удалось прочитать аудиодорожку видео.".to_string())?;
    let peaks = waveform_peaks_from_reader(stdout)?;
    let status = process
        .wait()
        .map_err(|error| format!("Не удалось завершить подготовку аудиодорожки: {error}"))?;

    if !status.success() || peaks.is_empty() {
        return Err("В исходном видео нет доступной аудиодорожки.".to_string());
    }

    Ok(AudioWaveform {
        source_path: source_path_string,
        source_file_size_bytes,
        source_modified_unix_ms,
        peaks_per_second: WAVEFORM_PEAKS_PER_SECOND,
        peaks,
    })
}

fn waveform_peaks_from_reader(mut reader: impl Read) -> Result<Vec<u16>, String> {
    let mut peaks = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut low_byte = None;
    let mut current_peak = 0_u16;
    let mut samples_in_peak = 0_usize;

    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Не удалось прочитать аудиодорожку: {error}"))?;
        if count == 0 {
            break;
        }

        append_waveform_peaks(
            &buffer[..count],
            &mut low_byte,
            &mut current_peak,
            &mut samples_in_peak,
            &mut peaks,
        );
    }

    if samples_in_peak > 0 {
        peaks.push(current_peak);
    }
    Ok(peaks)
}

fn append_waveform_peaks(
    bytes: &[u8],
    low_byte: &mut Option<u8>,
    current_peak: &mut u16,
    samples_in_peak: &mut usize,
    peaks: &mut Vec<u16>,
) {
    for byte in bytes {
        let Some(low) = low_byte.take() else {
            *low_byte = Some(*byte);
            continue;
        };

        let sample = i16::from_le_bytes([low, *byte]);
        let amplitude = i32::from(sample).unsigned_abs() as u16;
        *current_peak = (*current_peak).max(amplitude);
        *samples_in_peak += 1;
        if *samples_in_peak == WAVEFORM_SAMPLES_PER_PEAK {
            peaks.push(*current_peak);
            *current_peak = 0;
            *samples_in_peak = 0;
        }
    }
}

#[cfg(test)]
fn waveform_peaks_from_pcm(bytes: &[u8]) -> Vec<u16> {
    let mut peaks = Vec::new();
    let mut low_byte = None;
    let mut current_peak = 0_u16;
    let mut samples_in_peak = 0_usize;
    append_waveform_peaks(
        bytes,
        &mut low_byte,
        &mut current_peak,
        &mut samples_in_peak,
        &mut peaks,
    );
    if samples_in_peak > 0 {
        peaks.push(current_peak);
    }
    peaks
}

fn find_ffmpeg() -> Result<PathBuf, String> {
    find_external_tool(
        &[
            PathBuf::from("/opt/homebrew/bin/ffmpeg"),
            PathBuf::from("/usr/local/bin/ffmpeg"),
            PathBuf::from("ffmpeg"),
        ],
        "FFmpeg не найден. Установите FFmpeg, чтобы вырезать видео.",
    )
}

fn find_yt_dlp() -> Result<PathBuf, String> {
    find_external_tool(
        &[
            PathBuf::from("/opt/homebrew/bin/yt-dlp"),
            PathBuf::from("/usr/local/bin/yt-dlp"),
            PathBuf::from("yt-dlp"),
        ],
        "yt-dlp не найден. Установите yt-dlp, чтобы скачать видео по ссылке.",
    )
}

fn find_deno() -> Result<PathBuf, String> {
    find_external_tool(
        &[
            PathBuf::from("/opt/homebrew/bin/deno"),
            PathBuf::from("/usr/local/bin/deno"),
            PathBuf::from("deno"),
        ],
        "Deno не найден. Установите Deno, чтобы скачать видео по ссылке YouTube.",
    )
}

fn find_external_tool(candidates: &[PathBuf], error_message: &str) -> Result<PathBuf, String> {
    for candidate in candidates {
        if candidate == Path::new("ffmpeg")
            || candidate == Path::new("yt-dlp")
            || candidate == Path::new("deno")
            || candidate.is_file()
        {
            return Ok(candidate.clone());
        }
    }
    Err(error_message.to_string())
}

fn command_is_available(command: &Path, version_argument: &str) -> bool {
    Command::new(command)
        .arg(version_argument)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn deno_runtime_argument(deno: &Path) -> String {
    if deno == Path::new("deno") {
        "deno".to_string()
    } else {
        format!("deno:{}", deno.display())
    }
}

fn validate_youtube_url(value: &str) -> Result<(), String> {
    let Some(host_and_path) = value.trim().strip_prefix("https://") else {
        return Err("Укажите ссылку на видео YouTube, начинающуюся с https://.".to_string());
    };
    let host = host_and_path
        .split('/')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_youtube = matches!(
        host.as_str(),
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtu.be"
    );
    if is_youtube {
        Ok(())
    } else {
        Err("Укажите ссылку на видео YouTube.".to_string())
    }
}

fn fetch_youtube_video_title(
    yt_dlp: &Path,
    deno: &Path,
    url: &str,
    use_chrome_cookies: bool,
) -> Result<String, String> {
    let deno_runtime = deno_runtime_argument(deno);
    let mut command = Command::new(yt_dlp);
    command.args([
        "--no-playlist",
        "--skip-download",
        "--print",
        "%(title)s",
        "--js-runtimes",
        &deno_runtime,
        "--remote-components",
        "ejs:github",
    ]);
    if use_chrome_cookies {
        command.args(["--cookies-from-browser", "chrome"]);
    }
    let output = command
        .arg(url.trim())
        .output()
        .map_err(|error| format!("Не удалось запустить yt-dlp: {error}"))?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr);
        let details = details.trim();
        let details = if details.is_empty() {
            "yt-dlp завершился с ошибкой.".to_string()
        } else {
            details.chars().take(800).collect()
        };
        return Err(format!("Не удалось получить название видео: {details}"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let title = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .last()
        .ok_or_else(|| "yt-dlp не вернул название видео.".to_string())?;
    normalized_youtube_project_name(title)
}

fn normalized_youtube_project_name(value: &str) -> Result<String, String> {
    let name = value.trim().chars().take(120).collect::<String>();
    normalized_project_name(&name)
}

fn download_youtube_video_file(
    app: &AppHandle,
    draft_id: &str,
    yt_dlp: &Path,
    ffmpeg: &Path,
    deno: &Path,
    source_directory: &Path,
    url: &str,
    use_chrome_cookies: bool,
) -> Result<PathBuf, String> {
    fs::create_dir_all(source_directory)
        .map_err(|error| format!("Не удалось создать папку для загружаемого видео: {error}"))?;
    emit_youtube_download_progress(app, draft_id, "starting", None, None, None, None);
    let output_template = source_directory.join("%(title)s [%(id)s].%(ext)s");
    let deno_runtime = deno_runtime_argument(deno);
    let mut command = Command::new(yt_dlp);
    command
        .args([
            "--no-playlist",
            "--newline",
            "--progress-delta",
            "0.5",
            "--progress-template",
            "download:%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s",
            "--print",
            "after_move:filepath",
            "--js-runtimes",
            &deno_runtime,
            "--remote-components",
            "ejs:github",
            "-f",
            YOUTUBE_MP4_FORMAT,
            "--merge-output-format",
            "mp4",
        ])
        .arg("-o")
        .arg(&output_template);
    if let Some(ffmpeg_directory) = ffmpeg.parent().filter(|path| !path.as_os_str().is_empty()) {
        command.arg("--ffmpeg-location").arg(ffmpeg_directory);
    }
    if use_chrome_cookies {
        command.args(["--cookies-from-browser", "chrome"]);
    }
    let mut child = command
        .arg(url.trim())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Не удалось запустить yt-dlp: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Не удалось прочитать вывод yt-dlp.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Не удалось прочитать ошибки yt-dlp.".to_string())?;
    let stdout_app = app.clone();
    let stdout_draft_id = draft_id.to_owned();
    let stdout_thread = thread::spawn(move || {
        let mut output = Vec::new();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(progress) = parse_youtube_download_progress(&line, &stdout_draft_id) {
                let _ = stdout_app.emit("youtube-download-progress", progress);
            } else {
                output.push(line);
            }
        }
        output
    });
    let progress_app = app.clone();
    let progress_draft_id = draft_id.to_owned();
    let stderr_thread = thread::spawn(move || {
        let mut diagnostics = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(progress) = parse_youtube_download_progress(&line, &progress_draft_id) {
                let _ = progress_app.emit("youtube-download-progress", progress);
            } else if line.contains("[Merger]") || line.contains("Merging formats") {
                emit_youtube_download_progress(
                    &progress_app,
                    &progress_draft_id,
                    "merging",
                    None,
                    None,
                    None,
                    None,
                );
            } else if diagnostics.len() < 8_000 {
                diagnostics.push_str(&line);
                diagnostics.push('\n');
            }
        }
        diagnostics
    });
    let status = child
        .wait()
        .map_err(|error| format!("Не удалось дождаться yt-dlp: {error}"))?;
    let stdout = stdout_thread
        .join()
        .map_err(|_| "Не удалось прочитать результат yt-dlp.".to_string())?;
    let stderr = stderr_thread
        .join()
        .map_err(|_| "Не удалось прочитать ошибки yt-dlp.".to_string())?;
    if !status.success() {
        let details = stderr.trim();
        let details = if details.is_empty() {
            "yt-dlp завершился с ошибкой.".to_string()
        } else {
            details.chars().take(800).collect()
        };
        return Err(format!("Не удалось скачать видео: {details}"));
    }

    let downloaded_path = stdout
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .last()
        .map(PathBuf::from)
        .ok_or_else(|| "yt-dlp не сообщил путь к загруженному видео.".to_string())?;
    if !downloaded_path.is_file() {
        return Err("yt-dlp не сохранил загруженное видео.".to_string());
    }
    if !downloaded_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        return Err("yt-dlp сохранил не MP4-файл. Выберите другое видео или формат.".to_string());
    }
    Ok(downloaded_path)
}

fn emit_youtube_download_progress(
    app: &AppHandle,
    draft_id: &str,
    phase: &'static str,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    speed_bytes_per_second: Option<f64>,
    eta_seconds: Option<u64>,
) {
    let _ = app.emit(
        "youtube-download-progress",
        YoutubeDownloadProgress {
            draft_id: draft_id.to_owned(),
            phase,
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_second,
            eta_seconds,
        },
    );
}

fn parse_youtube_download_progress(line: &str, draft_id: &str) -> Option<YoutubeDownloadProgress> {
    let values = line
        .trim()
        .strip_prefix("download:")?
        .split('|')
        .collect::<Vec<_>>();
    if values.len() != 5 {
        return None;
    }
    let downloaded_bytes = parse_progress_u64(values[0]);
    let total_bytes = parse_progress_u64(values[1]).or_else(|| parse_progress_u64(values[2]));
    let speed_bytes_per_second = parse_progress_f64(values[3]);
    let eta_seconds = parse_progress_u64(values[4]);

    Some(YoutubeDownloadProgress {
        draft_id: draft_id.to_owned(),
        phase: "downloading",
        downloaded_bytes,
        total_bytes,
        speed_bytes_per_second,
        eta_seconds,
    })
}

fn parse_progress_u64(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

fn parse_progress_f64(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
}

fn openai_keyring_entry() -> Result<Entry, String> {
    Entry::new(OPENAI_KEYRING_SERVICE, OPENAI_KEYRING_ACCOUNT)
        .map_err(|error| format!("Не удалось открыть macOS Keychain: {error}"))
}

fn legacy_openai_keyring_entry() -> Result<Entry, String> {
    Entry::new(LEGACY_OPENAI_KEYRING_SERVICE, OPENAI_KEYRING_ACCOUNT)
        .map_err(|error| format!("Не удалось открыть macOS Keychain: {error}"))
}

fn delete_keyring_entry(entry: Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "Не удалось удалить ключ из macOS Keychain: {error}"
        )),
    }
}

fn ai_key_marker_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("ai-key-marker.json"))
        .map_err(|error| format!("Не удалось определить папку настроек: {error}"))
}

fn ai_settings_from_marker(app: &AppHandle) -> Result<AiSettings, String> {
    let key_status = match fs::read_to_string(ai_key_marker_path(app)?) {
        Ok(contents) => {
            let marker = serde_json::from_str::<AiKeyMarker>(&contents)
                .map_err(|error| format!("Не удалось прочитать настройки ключа: {error}"))?;
            if marker.api_key_configured {
                AiKeyStatus::Saved
            } else {
                AiKeyStatus::NotConfigured
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => AiKeyStatus::Unknown,
        Err(error) => return Err(format!("Не удалось прочитать настройки ключа: {error}")),
    };
    Ok(AiSettings {
        key_status,
        model: OPENAI_MODEL,
    })
}

fn save_ai_key_marker(app: &AppHandle, api_key_configured: bool) -> Result<(), String> {
    let path = ai_key_marker_path(app)?;
    let directory = path
        .parent()
        .ok_or_else(|| "Не удалось определить папку настроек.".to_string())?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Не удалось создать папку настроек: {error}"))?;
    let contents = serde_json::to_string(&AiKeyMarker { api_key_configured })
        .map_err(|error| format!("Не удалось записать настройки ключа: {error}"))?;
    fs::write(path, contents)
        .map_err(|error| format!("Не удалось записать настройки ключа: {error}"))
}

fn cached_openai_api_key(key_cache: &OpenAiKeyCache) -> Result<Option<String>, String> {
    key_cache
        .0
        .lock()
        .map(|cache| cache.clone())
        .map_err(|_| "Не удалось получить доступ к ключу OpenAI в памяти приложения.".to_string())
}

fn cache_openai_api_key(key_cache: &OpenAiKeyCache, api_key: String) -> Result<(), String> {
    let mut cache = key_cache
        .0
        .lock()
        .map_err(|_| "Не удалось сохранить ключ OpenAI в памяти приложения.".to_string())?;
    *cache = Some(api_key);
    Ok(())
}

fn clear_openai_api_key(key_cache: &OpenAiKeyCache) -> Result<(), String> {
    let mut cache = key_cache
        .0
        .lock()
        .map_err(|_| "Не удалось очистить ключ OpenAI из памяти приложения.".to_string())?;
    *cache = None;
    Ok(())
}

fn normalized_openai_api_key(value: &str) -> Result<String, String> {
    let key = value.trim();
    if key.is_empty() {
        return Err("Вставьте OpenAI API key.".to_string());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("OpenAI API key не должен содержать пробелы.".to_string());
    }
    Ok(key.to_string())
}

fn stored_openai_api_key(app: &AppHandle, key_cache: &OpenAiKeyCache) -> Result<String, String> {
    if let Some(api_key) = cached_openai_api_key(key_cache)? {
        return Ok(api_key);
    }

    let api_key = match read_keyring_password(openai_keyring_entry()?)? {
        Some(api_key) => Some(api_key),
        None => read_keyring_password(legacy_openai_keyring_entry()?)?,
    };
    match api_key {
        Some(api_key) => {
            cache_openai_api_key(key_cache, api_key.clone())?;
            save_ai_key_marker(app, true)?;
            Ok(api_key)
        }
        None => {
            save_ai_key_marker(app, false)?;
            Err("Добавьте OpenAI API key в AI Settings.".to_string())
        }
    }
}

fn read_keyring_password(entry: Entry) -> Result<Option<String>, String> {
    match entry.get_password() {
        Ok(api_key) if !api_key.trim().is_empty() => Ok(Some(api_key)),
        Ok(_) | Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(format!(
            "Не удалось прочитать OpenAI API key из macOS Keychain: {error}"
        )),
    }
}

fn normalized_ai_instruction(value: &str) -> Result<String, String> {
    let instruction = value.trim();
    if instruction.is_empty() {
        return Err("Опишите, какие моменты нужно найти.".to_string());
    }
    if instruction.chars().count() > 800 {
        return Err("Инструкция для ИИ не должна быть длиннее 800 символов.".to_string());
    }
    Ok(instruction.to_string())
}

async fn request_openai_subtitle_analysis(
    api_key: &str,
    instruction: &str,
    subtitles: &[SubtitleCue],
) -> Result<OpenAiAnalysisResponse, String> {
    let request_body = json!({
        "model": OPENAI_MODEL,
        "instructions": "Ты видеоредактор. Анализируй только переданные реплики субтитров. Выбирай до 12 отдельных реплик, соответствующих инструкции пользователя. Для каждого результата верни только source_cue_index существующей реплики и короткое пояснение note по-русски, почему этот момент интересен. Не возвращай таймкоды, интервалы или номера, которых нет в схеме: приложение само подставит настоящий таймкод из исходных субтитров.",
        "input": build_openai_subtitle_input(instruction, subtitles),
        "max_output_tokens": 3000,
        "text": {
            "format": {
                "type": "json_schema",
                "name": "subtitle_moment_analysis",
                "strict": true,
                "schema": openai_subtitle_response_schema(subtitles.len())
            }
        }
    });
    let response = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Не удалось подготовить запрос к OpenAI: {error}"))?
        .post("https://api.openai.com/v1/responses")
        .bearer_auth(api_key)
        .json(&request_body)
        .send()
        .await
        .map_err(|error| format!("Не удалось обратиться к OpenAI: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Не удалось прочитать ответ OpenAI: {error}"))?;
    if !status.is_success() {
        return Err(openai_error_message(status.as_u16(), &body));
    }
    let response_value = serde_json::from_str::<Value>(&body)
        .map_err(|error| format!("OpenAI вернул непонятный ответ: {error}"))?;
    let output_text = response_value
        .get("output_text")
        .and_then(Value::as_str)
        .or_else(|| {
            response_value
                .pointer("/output/0/content/0/text")
                .and_then(Value::as_str)
        })
        .ok_or_else(|| "OpenAI не вернул результат разбора субтитров.".to_string())?;
    serde_json::from_str(output_text)
        .map_err(|error| format!("OpenAI вернул неверную структуру разбора: {error}"))
}

fn openai_error_message(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(500).collect::<String>());
    let message = if message.trim().is_empty() {
        "без дополнительного текста".to_string()
    } else {
        message
    };
    format!("OpenAI вернул ошибку {status}: {message}")
}

fn openai_subtitle_response_schema(subtitle_count: usize) -> Value {
    debug_assert!(subtitle_count > 0);
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "moments": {
                "type": "array",
                "maxItems": 12,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "source_cue_index": {
                            "type": "integer",
                            "minimum": 0,
                            "maximum": subtitle_count.saturating_sub(1)
                        },
                        "note": { "type": "string", "maxLength": 180 }
                    },
                    "required": ["source_cue_index", "note"]
                }
            }
        },
        "required": ["moments"]
    })
}

fn build_openai_subtitle_input(instruction: &str, subtitles: &[SubtitleCue]) -> String {
    let cues = subtitles
        .iter()
        .enumerate()
        .map(|(index, cue)| {
            format!(
                "[{index}] {:.3}–{:.3} | {}",
                cue.start_seconds, cue.end_seconds, cue.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("Инструкция пользователя:\n{instruction}\n\nРеплики субтитров:\n{cues}")
}

fn validate_ai_moments(
    raw_moments: Vec<OpenAiMoment>,
    subtitles: &[SubtitleCue],
) -> Result<Vec<AiMoment>, String> {
    let mut moments = Vec::new();
    for raw_moment in raw_moments {
        if raw_moment.source_cue_index >= subtitles.len() {
            return Err("OpenAI указал реплику, которой нет в исходных субтитрах.".to_string());
        }
        let note = raw_moment.note.trim();
        if note.is_empty() {
            return Err("OpenAI вернул момент без пояснения.".to_string());
        }
        let cue = &subtitles[raw_moment.source_cue_index];
        let start_seconds = cue.start_seconds;
        let end_seconds = cue.end_seconds;
        if end_seconds <= start_seconds {
            return Err("В исходных субтитрах есть реплика с неверными границами.".to_string());
        }
        if moments
            .iter()
            .any(|moment: &AiMoment| moment.start_cue_index == raw_moment.source_cue_index)
        {
            continue;
        }
        moments.push(AiMoment {
            start_cue_index: raw_moment.source_cue_index,
            end_cue_index: raw_moment.source_cue_index,
            start_seconds,
            end_seconds,
            note: note.chars().take(180).collect(),
        });
    }
    moments.sort_by(|left, right| {
        left.start_seconds
            .partial_cmp(&right.start_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(moments)
}

fn subtitle_fingerprint(subtitles: &[SubtitleCue]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for cue in subtitles {
        for byte in cue
            .start_seconds
            .to_bits()
            .to_le_bytes()
            .into_iter()
            .chain(cue.end_seconds.to_bits().to_le_bytes())
            .chain(cue.text.as_bytes().iter().copied())
            .chain([0xff])
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("{hash:016x}")
}

fn format_seconds(seconds: f64) -> String {
    format!("{seconds:.3}")
}

fn filename_timecode(seconds: f64) -> String {
    let whole_seconds = seconds.max(0.0).floor() as u64;
    let hours = whole_seconds / 3600;
    let minutes = (whole_seconds % 3600) / 60;
    let seconds = whole_seconds % 60;
    format!("{hours:02}-{minutes:02}-{seconds:02}")
}

fn safe_clip_filename_stem(value: &str) -> String {
    const MAX_CHARACTERS: usize = 120;
    const MAX_UTF8_BYTES: usize = 220;
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut result = String::new();
    for character in cleaned.chars().take(MAX_CHARACTERS) {
        if result.len() + character.len_utf8() > MAX_UTF8_BYTES {
            break;
        }
        result.push(character);
    }
    result.trim_matches('.').trim().to_string()
}

fn safe_directory_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "project".to_string()
    } else {
        trimmed.to_string()
    }
}

fn projects_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("projects"))
        .map_err(|error| format!("Не удалось определить папку данных приложения: {error}"))
}

fn project_workspace_directory(app: &AppHandle, draft: &ProjectDraft) -> Result<PathBuf, String> {
    let base = app
        .path()
        .video_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|error| format!("Не удалось определить папку для файлов проекта: {error}"))?;
    let short_id = draft.id.get(..8).unwrap_or(&draft.id);
    Ok(base
        .join("SnipPaw")
        .join("projects")
        .join(format!("{}-{short_id}", safe_directory_name(&draft.name))))
}

fn project_directory(app: &AppHandle, project_id: &str) -> Result<PathBuf, String> {
    let project_id = Uuid::parse_str(project_id)
        .map_err(|_| "Не удалось найти указанный проект.".to_string())?;
    Ok(projects_root(app)?.join(project_id.to_string()))
}

fn inspect_video_file(path: &Path) -> Result<VideoFile, String> {
    if !path.is_file() {
        return Err("Выбранный файл не найден или не является видеофайлом.".to_string());
    }
    if !is_supported_video(path) {
        return Err("Поддерживаются только файлы MP4 с видео H.264 и звуком AAC.".to_string());
    }

    let metadata =
        fs::metadata(path).map_err(|error| format!("Не удалось прочитать видео: {error}"))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|error| format!("Не удалось определить путь к видео: {error}"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Не удалось прочитать имя видеофайла.".to_string())?;

    Ok(VideoFile {
        path: canonical_path.to_string_lossy().into_owned(),
        file_name: file_name.to_owned(),
        file_size_bytes: metadata.len(),
    })
}

fn is_supported_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            SUPPORTED_VIDEO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
        .unwrap_or(false)
}

fn is_supported_subtitle(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            SUPPORTED_SUBTITLE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
        .unwrap_or(false)
}

fn normalized_project_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() {
        return Err("Укажите название проекта.".to_string());
    }
    if name.chars().count() > 120 {
        return Err("Название проекта не должно быть длиннее 120 символов.".to_string());
    }
    Ok(name.to_owned())
}

fn now_unix_ms() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| format!("Не удалось получить текущее время: {error}"))
}

fn migrate_legacy_app_data(app: &AppHandle) -> Result<(), String> {
    let new_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Не удалось определить папку данных приложения: {error}"))?;
    let parent = new_root
        .parent()
        .ok_or_else(|| "Не удалось определить папку данных приложения.".to_string())?;
    let legacy_root = parent.join(LEGACY_APP_IDENTIFIER);
    migrate_legacy_app_data_root(&new_root, &legacy_root)
}

fn migrate_legacy_app_data_root(new_root: &Path, legacy_root: &Path) -> Result<(), String> {
    if new_root.exists() || !legacy_root.exists() {
        return Ok(());
    }

    fs::rename(legacy_root, new_root).map_err(|error| {
        format!("Не удалось перенести данные из предыдущей версии приложения: {error}")
    })
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            migrate_legacy_app_data(&app.handle())?;
            app.manage(PreviewServer::start()?);
            app.manage(OpenAiKeyCache::default());
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            inspect_video,
            preview_video_url,
            audio_waveform,
            list_projects,
            check_import_tools,
            prepare_project_import,
            discard_project_draft,
            complete_file_import,
            resolve_youtube_video_title,
            download_youtube_video,
            import_subtitles,
            get_ai_settings,
            save_openai_api_key,
            delete_openai_api_key,
            analyze_subtitles,
            delete_project,
            open_exports_folder,
            export_clip
        ])
        .run(tauri::generate_context!())
        .expect("Ошибка при запуске SnipPaw");
}

#[cfg(test)]
mod tests {
    use super::{
        build_openai_subtitle_input, cache_openai_api_key, cached_openai_api_key,
        clear_openai_api_key, delete_mp4_clips_in_directory, deno_runtime_argument,
        filename_timecode, is_supported_subtitle, is_supported_video, migrate_legacy_app_data_root,
        normalized_ai_instruction, normalized_project_name, normalized_youtube_project_name,
        openai_subtitle_response_schema, parse_byte_range, parse_subtitle_cues,
        parse_youtube_download_progress, safe_clip_filename_stem, safe_directory_name,
        subtitle_fingerprint, validate_ai_moments, validate_clip_bounds, validate_youtube_url,
        waveform_peaks_from_pcm, ExportQuality, OpenAiKeyCache, OpenAiMoment, PreviewServer,
        SubtitleCue, WAVEFORM_SAMPLES_PER_PEAK,
    };
    use std::{
        io::{Read, Write},
        net::TcpStream,
        path::Path,
    };
    use uuid::Uuid;

    #[test]
    fn accepts_only_mp4_video_extensions_case_insensitively() {
        assert!(is_supported_video(Path::new("stream.MP4")));
        assert!(!is_supported_video(Path::new("stream.webm")));
        assert!(!is_supported_video(Path::new("stream.mov")));
        assert!(!is_supported_video(Path::new("subtitles.srt")));
        assert!(is_supported_subtitle(Path::new("stream.SRT")));
        assert!(is_supported_subtitle(Path::new("stream.vtt")));
        assert!(!is_supported_subtitle(Path::new("stream.txt")));
    }

    #[test]
    fn groups_pcm_audio_into_compact_waveform_peaks() {
        let mut pcm = Vec::new();
        for _ in 0..WAVEFORM_SAMPLES_PER_PEAK {
            pcm.extend_from_slice(&1_200_i16.to_le_bytes());
        }
        for _ in 0..WAVEFORM_SAMPLES_PER_PEAK {
            pcm.extend_from_slice(&(-3_200_i16).to_le_bytes());
        }

        assert_eq!(waveform_peaks_from_pcm(&pcm), vec![1_200, 3_200]);
    }

    #[test]
    fn parses_srt_and_vtt_cue_boundaries_and_text() {
        let srt = "1\n00:45:01,125 --> 00:45:08,600\n<i>Фэнтези</i>, лично в моём\nпонимании\n\n2\n00:47:52,000 --> 00:47:56,000\nВторая фраза";
        let cues = parse_subtitle_cues(srt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_seconds, 2701.125);
        assert_eq!(cues[0].end_seconds, 2708.6);
        assert_eq!(cues[0].text, "Фэнтези, лично в моём понимании");

        let vtt = "WEBVTT\n\n00:00:05.250 --> 00:00:07.500 align:start\nПервая реплика";
        let cues = parse_subtitle_cues(vtt).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_seconds, 5.25);
        assert_eq!(cues[0].end_seconds, 7.5);
    }

    #[test]
    fn trims_project_name_and_rejects_empty_value() {
        assert_eq!(
            normalized_project_name("  Распаковка 4  ").unwrap(),
            "Распаковка 4"
        );
        assert!(normalized_project_name("   ").is_err());
    }

    #[test]
    fn uses_youtube_title_as_a_valid_limited_project_name() {
        assert_eq!(
            normalized_youtube_project_name("  Небольшой обзор Steam Deck  ").unwrap(),
            "Небольшой обзор Steam Deck"
        );
        assert_eq!(
            normalized_youtube_project_name(&"я".repeat(121))
                .unwrap()
                .chars()
                .count(),
            120
        );
        assert!(normalized_youtube_project_name("   ").is_err());
    }

    #[test]
    fn validates_ai_moments_against_existing_subtitle_indices() {
        let subtitles = vec![
            SubtitleCue {
                start_seconds: 10.0,
                end_seconds: 12.0,
                text: "Первая реплика".to_string(),
            },
            SubtitleCue {
                start_seconds: 12.0,
                end_seconds: 16.0,
                text: "Вторая реплика".to_string(),
            },
        ];
        let moments = validate_ai_moments(
            vec![OpenAiMoment {
                source_cue_index: 1,
                note: "Ясная самостоятельная мысль".to_string(),
            }],
            &subtitles,
        )
        .unwrap();
        assert_eq!(moments.len(), 1);
        assert_eq!(moments[0].start_seconds, 12.0);
        assert_eq!(moments[0].end_seconds, 16.0);
        assert_eq!(moments[0].note, "Ясная самостоятельная мысль");
        assert!(validate_ai_moments(
            vec![OpenAiMoment {
                source_cue_index: 2,
                note: "Несуществующая реплика".to_string(),
            }],
            &subtitles,
        )
        .is_err());
    }

    #[test]
    fn restricts_openai_to_existing_subtitle_references() {
        let schema = openai_subtitle_response_schema(3);
        let source_cue_index = schema
            .pointer("/properties/moments/items/properties/source_cue_index")
            .unwrap();
        assert_eq!(
            source_cue_index
                .get("minimum")
                .and_then(|value| value.as_u64()),
            Some(0)
        );
        assert_eq!(
            source_cue_index
                .get("maximum")
                .and_then(|value| value.as_u64()),
            Some(2)
        );
        assert!(schema
            .pointer("/properties/moments/items/properties/start_cue_index")
            .is_none());
        assert!(schema
            .pointer("/properties/moments/items/properties/end_cue_index")
            .is_none());
    }

    #[test]
    fn caches_the_openai_key_only_for_the_running_application() {
        let cache = OpenAiKeyCache::default();
        assert_eq!(cached_openai_api_key(&cache).unwrap(), None);

        cache_openai_api_key(&cache, "test-key".to_string()).unwrap();
        assert_eq!(
            cached_openai_api_key(&cache).unwrap(),
            Some("test-key".to_string())
        );

        clear_openai_api_key(&cache).unwrap();
        assert_eq!(cached_openai_api_key(&cache).unwrap(), None);
    }

    #[test]
    fn makes_a_stable_subtitle_input_and_fingerprint() {
        let subtitles = vec![SubtitleCue {
            start_seconds: 1.0,
            end_seconds: 2.0,
            text: "Тестовая реплика".to_string(),
        }];
        let input = build_openai_subtitle_input("Ищи шутки", &subtitles);
        assert!(input.contains("[0] 1.000–2.000 | Тестовая реплика"));
        assert_eq!(
            subtitle_fingerprint(&subtitles),
            subtitle_fingerprint(&subtitles)
        );
        assert!(normalized_ai_instruction("  Ищи шутки  ").is_ok());
        assert!(normalized_ai_instruction("  ").is_err());
    }

    #[test]
    fn accepts_standard_youtube_urls_only() {
        assert!(validate_youtube_url("https://www.youtube.com/watch?v=abc").is_ok());
        assert!(validate_youtube_url("https://youtu.be/abc").is_ok());
        assert!(validate_youtube_url("http://www.youtube.com/watch?v=abc").is_err());
        assert!(validate_youtube_url("https://example.com/watch?v=abc").is_err());
    }

    #[test]
    fn uses_an_explicit_deno_runtime_path_when_available() {
        assert_eq!(deno_runtime_argument(Path::new("deno")), "deno");
        assert_eq!(
            deno_runtime_argument(Path::new("/opt/homebrew/bin/deno")),
            "deno:/opt/homebrew/bin/deno"
        );
    }

    #[test]
    fn parses_youtube_download_progress_from_ytdlp_template() {
        let progress =
            parse_youtube_download_progress("download:262144|1048576|NA|524288.0|2", "draft-id")
                .unwrap();
        assert_eq!(progress.draft_id, "draft-id");
        assert_eq!(progress.phase, "downloading");
        assert_eq!(progress.downloaded_bytes, Some(262_144));
        assert_eq!(progress.total_bytes, Some(1_048_576));
        assert_eq!(progress.speed_bytes_per_second, Some(524_288.0));
        assert_eq!(progress.eta_seconds, Some(2));
        assert!(parse_youtube_download_progress("[download] 42%", "draft-id").is_none());
    }

    #[test]
    fn validates_clip_bounds_and_makes_safe_export_names() {
        assert!(validate_clip_bounds(10.0, 10.1).is_ok());
        assert!(validate_clip_bounds(10.0, 10.05).is_err());
        assert!(validate_clip_bounds(-1.0, 5.0).is_err());
        assert_eq!(filename_timecode(3723.8), "01-02-03");
        assert_eq!(safe_directory_name("Распаковка: #4!"), "Распаковка___4");
        assert_eq!(
            safe_clip_filename_stem("  Фэнтези: \"сказка\" / тест?  "),
            "Фэнтези сказка тест"
        );
        let long_name = safe_clip_filename_stem(&"я".repeat(121));
        assert!(long_name.chars().count() <= 120);
        assert!(long_name.len() <= 220);
    }

    #[test]
    fn uses_expected_ffmpeg_settings_for_each_export_quality() {
        assert_eq!(ExportQuality::High.ffmpeg_settings(), ("20", "192k"));
        assert_eq!(ExportQuality::Balanced.ffmpeg_settings(), ("23", "160k"));
        assert_eq!(ExportQuality::Compact.ffmpeg_settings(), ("28", "128k"));
    }

    #[test]
    fn deletes_only_mp4_files_from_an_export_folder() {
        let directory = std::env::temp_dir().join(format!("videocut-clips-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mp4_clip = directory.join("clip.mp4");
        let uppercase_mp4_clip = directory.join("second.MP4");
        let non_clip = directory.join("notes.txt");
        std::fs::write(&mp4_clip, b"clip").unwrap();
        std::fs::write(&uppercase_mp4_clip, b"clip").unwrap();
        std::fs::write(&non_clip, b"keep").unwrap();

        assert_eq!(delete_mp4_clips_in_directory(&directory).unwrap(), 2);
        assert!(!mp4_clip.exists());
        assert!(!uppercase_mp4_clip.exists());
        assert!(non_clip.exists());

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn migrates_legacy_app_data_without_overwriting_existing_new_data() {
        let root = std::env::temp_dir().join(format!("snippaw-migration-{}", Uuid::new_v4()));
        let legacy_root = root.join("com.videocut.desktop");
        let new_root = root.join("com.snippaw.desktop");
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::write(legacy_root.join("project.json"), b"legacy project").unwrap();

        migrate_legacy_app_data_root(&new_root, &legacy_root).unwrap();
        assert_eq!(
            std::fs::read(new_root.join("project.json")).unwrap(),
            b"legacy project"
        );
        assert!(!legacy_root.exists());

        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::write(legacy_root.join("project.json"), b"another project").unwrap();
        migrate_legacy_app_data_root(&new_root, &legacy_root).unwrap();
        assert_eq!(
            std::fs::read(new_root.join("project.json")).unwrap(),
            b"legacy project"
        );
        assert!(legacy_root.exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_http_ranges_for_media_playback() {
        assert_eq!(parse_byte_range("bytes=3-6", 10), Some((3, 6)));
        assert_eq!(parse_byte_range("bytes=3-", 10), Some((3, 9)));
        assert_eq!(parse_byte_range("bytes=-4", 10), Some((6, 9)));
        assert_eq!(parse_byte_range("bytes=10-", 10), None);
        assert_eq!(parse_byte_range("bytes=-0", 10), None);
    }

    #[test]
    fn preview_server_streams_a_requested_byte_range() {
        let source_path =
            std::env::temp_dir().join(format!("videocut-preview-{}.mp4", Uuid::new_v4()));
        std::fs::write(&source_path, b"0123456789").unwrap();

        let server = PreviewServer::start().unwrap();
        let url = server.register(source_path.clone()).unwrap();
        let target = url.strip_prefix("http://").unwrap();
        let (address, request_path) = target.split_once('/').unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        write!(
            stream,
            "GET /{request_path} HTTP/1.1\r\nHost: {address}\r\nRange: bytes=3-6\r\n\r\n"
        )
        .unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("HTTP/1.1 206 Partial Content"));
        assert!(headers.contains("Content-Type: video/mp4"));
        assert!(headers.contains("Content-Range: bytes 3-6/10"));
        assert_eq!(body, "3456");

        std::fs::remove_file(source_path).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires access to the local macOS Keychain"]
    fn macos_keychain_round_trips_a_generic_password() {
        let service = format!("com.videocut.desktop.keychain-probe-{}", Uuid::new_v4());
        let account = "probe";
        let entry = keyring::Entry::new(&service, account).unwrap();
        entry.set_password("videocut-keychain-probe").unwrap();
        drop(entry);

        let reloaded_entry = keyring::Entry::new(&service, account).unwrap();
        let result = reloaded_entry.get_password();
        let _ = reloaded_entry.delete_credential();

        assert_eq!(result.unwrap(), "videocut-keychain-probe");
    }
}
