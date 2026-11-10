import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { FormEvent, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { ProjectEditor } from "./ProjectEditor";
import type { AiSettings, ImportToolsStatus, Project, ProjectDeletion, ProjectDraft, VideoFile, YoutubeDownloadProgress, YoutubeVideoInfo } from "./types";
import "./styles.css";

const videoFilters = [
  { name: "MP4", extensions: ["mp4"] }
];

type ProjectDeleteMode = "project" | "files";
type CreateProjectStep = "source" | "file-name";
type ImportMethod = "file" | "youtube";

function TrashIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 7h16M10 11v6M14 11v6M9 7V4h6v3M6.5 7l.8 13h9.4l.8-13" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m5 12 4.2 4.2L19 6.8" />
    </svg>
  );
}

function ClipsFolderIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3.5 7.5h6l1.8 2h9.2v8.8a1.7 1.7 0 0 1-1.7 1.7H5.2a1.7 1.7 0 0 1-1.7-1.7z" />
      <path d="M12 13h5M14.5 10.5 17 13l-2.5 2.5" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.1 2.1-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.55v.1h-3v-.1a1.7 1.7 0 0 0-1.03-1.55 1.7 1.7 0 0 0-1.88.34l-.06.06-2.1-2.1.06-.06A1.7 1.7 0 0 0 7.06 15a1.7 1.7 0 0 0-1.55-1.03h-.1v-3h.1A1.7 1.7 0 0 0 7.06 9.94a1.7 1.7 0 0 0-.34-1.88L6.66 8l2.1-2.1.06.06a1.7 1.7 0 0 0 1.88.34 1.7 1.7 0 0 0 1.03-1.55v-.1h3v.1a1.7 1.7 0 0 0 1.03 1.55 1.7 1.7 0 0 0 1.88-.34l.06-.06 2.1 2.1-.06.06a1.7 1.7 0 0 0-.34 1.88 1.7 1.7 0 0 0 1.55 1.03h.1v3h-.1A1.7 1.7 0 0 0 19.4 15Z" />
    </svg>
  );
}

function formatSize(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} КБ`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(0)} МБ`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} ГБ`;
}

function formatDuration(seconds: number) {
  const rounded = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(rounded / 60);
  const remainingSeconds = rounded % 60;
  return minutes > 0 ? `${minutes}:${remainingSeconds.toString().padStart(2, "0")}` : `${remainingSeconds} с`;
}

function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [hasLoadedProjects, setLoadedProjects] = useState(false);
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [isDialogOpen, setDialogOpen] = useState(false);
  const [name, setName] = useState("");
  const [video, setVideo] = useState<VideoFile | null>(null);
  const [createStep, setCreateStep] = useState<CreateProjectStep>("source");
  const [draft, setDraft] = useState<ProjectDraft | null>(null);
  const [importMethod, setImportMethod] = useState<ImportMethod>("file");
  const [copySourceVideo, setCopySourceVideo] = useState(false);
  const [youtubeUrl, setYoutubeUrl] = useState("");
  const [useChromeCookies, setUseChromeCookies] = useState(false);
  const [importTools, setImportTools] = useState<ImportToolsStatus | null>(null);
  const [youtubeDownloadProgress, setYoutubeDownloadProgress] = useState<YoutubeDownloadProgress | null>(null);
  const [isSaving, setSaving] = useState(false);
  const [dialogError, setDialogError] = useState<string | null>(null);
  const [libraryError, setLibraryError] = useState<string | null>(null);
  const [projectToDelete, setProjectToDelete] = useState<Project | null>(null);
  const [deleteMode, setDeleteMode] = useState<ProjectDeleteMode>("project");
  const [deleteSourceVideo, setDeleteSourceVideo] = useState(true);
  const [deleteExportedClips, setDeleteExportedClips] = useState(false);
  const [isDeleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [openingExportsProjectId, setOpeningExportsProjectId] = useState<string | null>(null);
  const [isAiSettingsOpen, setAiSettingsOpen] = useState(false);
  const [aiSettings, setAiSettings] = useState<AiSettings | null>(null);
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [aiSettingsError, setAiSettingsError] = useState<string | null>(null);
  const [isSavingAiSettings, setSavingAiSettings] = useState(false);
  const [aiKeyStatusVersion, setAiKeyStatusVersion] = useState(0);

  useEffect(() => {
    void invoke<Project[]>("list_projects")
      .then((loadedProjects) => {
        setProjects(loadedProjects);
        setLoadedProjects(true);
        if (loadedProjects.length === 0) showCreateDialog();
      })
      .catch((reason: unknown) => setLibraryError(String(reason)));
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<YoutubeDownloadProgress>("youtube-download-progress", (event) => {
      setYoutubeDownloadProgress(event.payload);
    }).then((dispose) => {
      unlisten = dispose;
    });
    return () => unlisten?.();
  }, []);

  function showCreateDialog() {
    setName("");
    setVideo(null);
    setCreateStep("source");
    setDraft(null);
    setImportMethod("file");
    setCopySourceVideo(false);
    setYoutubeUrl("");
    setUseChromeCookies(false);
    setImportTools(null);
    setYoutubeDownloadProgress(null);
    setDialogError(null);
    setDialogOpen(true);
    void invoke<ImportToolsStatus>("check_import_tools")
      .then(setImportTools)
      .catch(() => setImportTools({ yt_dlp_available: false, ffmpeg_available: false, deno_available: false }));
  }

  async function discardDraft() {
    if (!draft) return;
    await invoke("discard_project_draft", { draftId: draft.id });
  }

  async function closeCreateDialog() {
    if (isSaving) return;
    setDialogError(null);
    try {
      await discardDraft();
      setDialogOpen(false);
      setDraft(null);
      setYoutubeDownloadProgress(null);
    } catch (reason) {
      setDialogError(String(reason));
    }
  }

  async function returnToImportSource() {
    if (isSaving) return;

    setSaving(true);
    setDialogError(null);
    try {
      await discardDraft();
      setDraft(null);
      setVideo(null);
      setName("");
      setYoutubeUrl("");
      setYoutubeDownloadProgress(null);
      setCreateStep("source");
    } catch (reason) {
      setDialogError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  function showDeleteDialog(project: Project) {
    setProjectToDelete(project);
    setDeleteMode("project");
    setDeleteSourceVideo(true);
    setDeleteExportedClips(false);
    setDeleteError(null);
  }

  async function deleteProject() {
    if (!projectToDelete) return;

    setDeleting(true);
    setDeleteError(null);
    try {
      await invoke<ProjectDeletion>("delete_project", {
        projectId: projectToDelete.id,
        deleteSourceVideo: deleteMode === "files" && deleteSourceVideo,
        deleteExportedClips: deleteMode === "files" && deleteExportedClips
      });
      setProjects((current) => current.filter((project) => project.id !== projectToDelete.id));
      setProjectToDelete(null);
    } catch (reason) {
      setDeleteError(String(reason));
    } finally {
      setDeleting(false);
    }
  }

  async function openExportsFolder(project: Project) {
    setOpeningExportsProjectId(project.id);
    setLibraryError(null);
    try {
      await invoke("open_exports_folder", { projectId: project.id });
    } catch (reason) {
      setLibraryError(String(reason));
    } finally {
      setOpeningExportsProjectId(null);
    }
  }

  async function openAiSettings() {
    setAiSettingsOpen(true);
    setAiSettings(null);
    setApiKeyInput("");
    setAiSettingsError(null);
    try {
      setAiSettings(await invoke<AiSettings>("get_ai_settings"));
    } catch (reason) {
      setAiSettingsError(String(reason));
    }
  }

  async function saveOpenAiApiKey() {
    if (!apiKeyInput.trim()) return;

    setSavingAiSettings(true);
    setAiSettingsError(null);
    try {
      const savedSettings = await invoke<AiSettings>("save_openai_api_key", { apiKey: apiKeyInput });
      setAiSettings(savedSettings);
      setApiKeyInput("");
      setAiKeyStatusVersion((version) => version + 1);
    } catch (reason) {
      setAiSettingsError(String(reason));
    } finally {
      setSavingAiSettings(false);
    }
  }

  async function deleteOpenAiApiKey() {
    setSavingAiSettings(true);
    setAiSettingsError(null);
    try {
      setAiSettings(await invoke<AiSettings>("delete_openai_api_key"));
      setApiKeyInput("");
      setAiKeyStatusVersion((version) => version + 1);
    } catch (reason) {
      setAiSettingsError(String(reason));
    } finally {
      setSavingAiSettings(false);
    }
  }

  async function selectVideo() {
    setDialogError(null);
    const selected = await open({ multiple: false, directory: false, filters: videoFilters });
    if (!selected || Array.isArray(selected)) return;

    try {
      const file = await invoke<VideoFile>("inspect_video", { path: selected });
      setVideo(file);
      setCreateStep("file-name");
    } catch (reason) {
      setDialogError(String(reason));
    }
  }

  async function completeFileImport(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!video || !name.trim()) return;

    setSaving(true);
    setDialogError(null);
    try {
      const activeDraft = draft ?? await invoke<ProjectDraft>("prepare_project_import", { name: name.trim() });
      setDraft(activeDraft);
      const project = await invoke<Project>("complete_file_import", {
        draftId: activeDraft.id,
        videoPath: video.path,
        copySourceVideo
      });
      setProjects((current) => [project, ...current]);
      setDialogOpen(false);
      setDraft(null);
      setYoutubeDownloadProgress(null);
      setSelectedProject(project);
    } catch (reason) {
      setDialogError(String(reason));
      setYoutubeDownloadProgress(null);
    } finally {
      setSaving(false);
    }
  }

  async function completeYoutubeImport(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!youtubeUrl.trim() || !importTools?.yt_dlp_available || !importTools.ffmpeg_available || !importTools.deno_available) return;

    setSaving(true);
    setDialogError(null);
    try {
      let activeDraft = draft;
      if (!activeDraft) {
        const videoInfo = await invoke<YoutubeVideoInfo>("resolve_youtube_video_title", {
          url: youtubeUrl.trim(),
          useChromeCookies
        });
        setName(videoInfo.title);
        activeDraft = await invoke<ProjectDraft>("prepare_project_import", { name: videoInfo.title });
        setDraft(activeDraft);
      }
      setYoutubeDownloadProgress({
        draft_id: activeDraft.id,
        phase: "starting",
        downloaded_bytes: null,
        total_bytes: null,
        speed_bytes_per_second: null,
        eta_seconds: null
      });
      const project = await invoke<Project>("download_youtube_video", {
        draftId: activeDraft.id,
        url: youtubeUrl.trim(),
        useChromeCookies
      });
      setProjects((current) => [project, ...current]);
      setDialogOpen(false);
      setDraft(null);
      setYoutubeDownloadProgress(null);
      setSelectedProject(project);
    } catch (reason) {
      setDialogError(String(reason));
      setYoutubeDownloadProgress(null);
    } finally {
      setSaving(false);
    }
  }

  const youtubeImportAvailable = Boolean(importTools?.yt_dlp_available && importTools.ffmpeg_available && importTools.deno_available);
  const activeYoutubeProgress = draft && youtubeDownloadProgress?.draft_id === draft.id
    ? youtubeDownloadProgress
    : null;
  const youtubeDownloadTotal = activeYoutubeProgress?.total_bytes ?? null;
  const youtubeDownloadedBytes = activeYoutubeProgress?.downloaded_bytes ?? null;
  const youtubeDownloadPercent = youtubeDownloadTotal !== null && youtubeDownloadedBytes !== null
    ? Math.min(100, Math.round((youtubeDownloadedBytes / youtubeDownloadTotal) * 100))
    : null;

  if (selectedProject) {
    return (
      <>
        <ProjectEditor
          key={selectedProject.id}
          project={selectedProject}
          onBack={() => setSelectedProject(null)}
          onOpenAiSettings={() => void openAiSettings()}
          aiKeyStatusVersion={aiKeyStatusVersion}
          onProjectUpdated={(updatedProject) => {
            setSelectedProject(updatedProject);
            setProjects((current) => current.map((project) => (
              project.id === updatedProject.id ? updatedProject : project
            )));
          }}
        />
        <AiSettingsDialog
          aiSettings={aiSettings}
          apiKeyInput={apiKeyInput}
          error={aiSettingsError}
          isOpen={isAiSettingsOpen}
          isSaving={isSavingAiSettings}
          onClose={() => !isSavingAiSettings && setAiSettingsOpen(false)}
          onDelete={() => void deleteOpenAiApiKey()}
          onInputChange={setApiKeyInput}
          onSave={() => void saveOpenAiApiKey()}
        />
      </>
    );
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand"><img className="brand-mark" src="/snippaw-icon.png" alt="" /> SnipPaw</div>
        <div className="topbar-actions">
          <button className="topbar-icon-button" type="button" title="Настройки" aria-label="Настройки" onClick={() => void openAiSettings()}><SettingsIcon /></button>
          <button className="new-project-button" type="button" title="Создать проект" aria-label="Создать проект" onClick={showCreateDialog}>＋</button>
        </div>
      </header>

      <section className="content">
        <div className="section-heading">
          <div>
            <h1>Мои проекты</h1>
          </div>
          {projects.length > 0 && <span className="project-count">{projects.length}</span>}
        </div>

        {libraryError && <p className="form-error" role="alert">{libraryError}</p>}

        {hasLoadedProjects && projects.length === 0 && !isDialogOpen && (
          <p className="empty-library-notice">Пока тут пусто. Нажмите +, чтобы создать новый проект.</p>
        )}

        {projects.length > 0 && (
          <div className="projects-table-wrap">
            <table className="projects-table">
              <thead>
                <tr><th scope="col">Проект</th><th scope="col">Исходное видео</th><th scope="col">Размер</th><th scope="col"><span className="visually-hidden">Действия</span></th></tr>
              </thead>
              <tbody>
                {projects.map((project) => (
                  <tr key={project.id}>
                    <td><button className="project-open-button" type="button" onClick={() => setSelectedProject(project)}>{project.name}</button></td>
                    <td className="project-source-name">{project.video.file_name}</td>
                    <td className="project-size">{formatSize(project.video.file_size_bytes)}</td>
                    <td className="project-row-actions">
                      <button className="project-clips-button" type="button" title="Открыть папку с вырезанными клипами в Finder" aria-label={`Открыть клипы проекта «${project.name}» в Finder`} disabled={openingExportsProjectId === project.id} onClick={() => void openExportsFolder(project)}>
                        <ClipsFolderIcon />
                      </button>
                      <button className="project-delete-button" type="button" title="Удалить проект" aria-label={`Удалить проект «${project.name}»`} onClick={() => showDeleteDialog(project)}>
                        <TrashIcon />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {isDialogOpen && (
        <div className="backdrop" role="presentation" onMouseDown={() => void closeCreateDialog()}>
          <form
            className="dialog"
            onSubmit={createStep === "file-name" ? completeFileImport : completeYoutubeImport}
            onMouseDown={(event) => event.stopPropagation()}
          >
            <div className="dialog-heading">
              <div>
                <p className="eyebrow">{createStep === "file-name" ? "Название проекта" : "Импорт видео"}</p>
              </div>
              <button className="icon-button" type="button" aria-label="Закрыть" disabled={isSaving} onClick={() => void closeCreateDialog()}>×</button>
            </div>

            {createStep === "file-name" ? (
              <>
                {video && (
                  <div className="selected-video">
                    <div className="video-file-icon">▶</div>
                    <div>
                      <strong>{video.file_name}</strong>
                      <span>{formatSize(video.file_size_bytes)} · {copySourceVideo ? "будет скопирован в папку проекта" : "останется в исходной папке"}</span>
                    </div>
                    <button className="text-button" type="button" disabled={isSaving} onClick={() => void selectVideo()}>Заменить</button>
                  </div>
                )}
                <label>
                  Название проекта
                  <input
                    autoFocus
                    maxLength={120}
                    placeholder="Введите название"
                    value={name}
                    disabled={isSaving}
                    onChange={(event) => setName(event.target.value)}
                  />
                </label>
                <label className="copy-source-option">
                  <input type="checkbox" checked={copySourceVideo} disabled={isSaving} onChange={(event) => setCopySourceVideo(event.target.checked)} />
                  <span>Скопировать исходное видео в папку проекта</span>
                </label>

                {dialogError && <p className="form-error" role="alert">{dialogError}</p>}

                <div className="dialog-actions">
                  <button className="secondary-button" type="button" disabled={isSaving} onClick={() => void returnToImportSource()}>Назад</button>
                  <button className="primary-button" type="submit" disabled={!name.trim() || isSaving}>
                    {isSaving ? "Создаём…" : "Создать проект"}
                  </button>
                </div>
              </>
            ) : (
              <>
                <div className="import-methods" role="radiogroup" aria-label="Способ импорта">
                  <button
                    type="button"
                    role="radio"
                    aria-checked={importMethod === "file"}
                    className={importMethod === "file" ? "import-method is-selected" : "import-method"}
                    disabled={isSaving}
                    onClick={() => setImportMethod("file")}
                  >
                    <strong>Файл с компьютера</strong>
                    <small>MP4</small>
                  </button>
                  <button
                    type="button"
                    role="radio"
                    aria-checked={importMethod === "youtube"}
                    className={importMethod === "youtube" ? "import-method is-selected" : "import-method"}
                    disabled={isSaving || !youtubeImportAvailable}
                    onClick={() => setImportMethod("youtube")}
                  >
                    <strong>Ссылка YouTube</strong>
                    <small>Скачать через yt-dlp</small>
                  </button>
                </div>

                {!importTools ? (
                  <p className="import-tool-status">Проверяем yt-dlp, FFmpeg и Deno…</p>
                ) : !youtubeImportAvailable && (
                  <p className="import-tool-status" role="status">
                    Импорт по ссылке недоступен: {!importTools.yt_dlp_available && "yt-dlp не установлен"}{!importTools.yt_dlp_available && (!importTools.ffmpeg_available || !importTools.deno_available) && ", "}{!importTools.ffmpeg_available && "FFmpeg не установлен"}{!importTools.ffmpeg_available && !importTools.deno_available && ", "}{!importTools.deno_available && "Deno не установлен"}.
                  </p>
                )}

                {importMethod === "file" ? (
                  <button className="file-picker" type="button" disabled={isSaving} onClick={() => void selectVideo()}>
                    <span>＋</span>
                    <strong>Выбрать файл</strong>
                  </button>
                ) : (
                  <>
                    <label>
                      Ссылка YouTube
                      <input
                        autoFocus
                        type="url"
                        placeholder="https://www.youtube.com/watch?v=…"
                        value={youtubeUrl}
                        disabled={isSaving || !youtubeImportAvailable}
                        onChange={(event) => setYoutubeUrl(event.target.value)}
                      />
                    </label>
                    <label className="copy-source-option">
                      <input type="checkbox" checked={useChromeCookies} disabled={isSaving || !youtubeImportAvailable} onChange={(event) => setUseChromeCookies(event.target.checked)} />
                      <span>Использовать cookies из Chrome</span>
                    </label>
                    <p className="import-helper">Видео будет скачано в папку проекта как MP4 (H.264/AAC).</p>
                    {isSaving && activeYoutubeProgress && (
                      <section className="youtube-download-progress" aria-live="polite">
                        {activeYoutubeProgress.phase === "merging" ? (
                          <strong>Объединяем видео и звук в MP4…</strong>
                        ) : activeYoutubeProgress.phase === "starting" ? (
                          <strong>Подготавливаем загрузку…</strong>
                        ) : (
                          <>
                            <div className="youtube-download-progress-heading">
                              <strong>Скачивание{youtubeDownloadPercent !== null ? ` · ${youtubeDownloadPercent}%` : "…"}</strong>
                              {youtubeDownloadPercent !== null && <span>{youtubeDownloadPercent}%</span>}
                            </div>
                            <div className="youtube-download-progress-track" role="progressbar" aria-label="Прогресс загрузки видео" aria-valuemin={0} aria-valuemax={100} aria-valuenow={youtubeDownloadPercent ?? undefined}>
                              <span style={{ width: `${youtubeDownloadPercent ?? 12}%` }} />
                            </div>
                            <p>
                              {activeYoutubeProgress.downloaded_bytes !== null && formatSize(activeYoutubeProgress.downloaded_bytes)}
                              {youtubeDownloadTotal !== null && ` из ${formatSize(youtubeDownloadTotal)}`}
                              {activeYoutubeProgress.speed_bytes_per_second !== null && ` · ${formatSize(activeYoutubeProgress.speed_bytes_per_second)}/с`}
                              {activeYoutubeProgress.eta_seconds !== null && ` · осталось ${formatDuration(activeYoutubeProgress.eta_seconds)}`}
                            </p>
                          </>
                        )}
                      </section>
                    )}
                  </>
                )}

                {dialogError && <p className="form-error" role="alert">{dialogError}</p>}

                <div className="dialog-actions">
                  <button className="secondary-button" type="button" disabled={isSaving} onClick={() => void closeCreateDialog()}>Отмена</button>
                  {importMethod === "youtube" && (
                    <button className="primary-button" type="submit" disabled={isSaving || !youtubeUrl.trim() || !youtubeImportAvailable}>
                      {isSaving ? (draft ? "Скачиваем…" : "Получаем название…") : "Скачать видео"}
                    </button>
                  )}
                </div>
              </>
            )}
          </form>
        </div>
      )}

      {projectToDelete && (
        <div className="backdrop" role="presentation" onMouseDown={() => !isDeleting && setProjectToDelete(null)}>
          <section className="dialog delete-dialog" role="dialog" aria-modal="true" aria-labelledby="delete-project-title" onMouseDown={(event) => event.stopPropagation()}>
            <div className="dialog-heading">
              <div>
                <p className="eyebrow">Удаление проекта</p>
                <h2 id="delete-project-title">Удалить «{projectToDelete.name}»?</h2>
              </div>
              <button className="icon-button" type="button" aria-label="Закрыть" disabled={isDeleting} onClick={() => setProjectToDelete(null)}>×</button>
            </div>

            <div className="delete-options" role="radiogroup" aria-label="Что удалить">
              <label className={deleteMode === "project" ? "delete-option is-selected" : "delete-option"}>
                <input type="radio" name="project-delete-mode" value="project" checked={deleteMode === "project"} disabled={isDeleting} onChange={() => setDeleteMode("project")} />
                <span><strong>Удалить только проект</strong><small>Исходное видео и нарезанные клипы останутся на диске.</small></span>
              </label>
              <label className={deleteMode === "files" ? "delete-option is-selected" : "delete-option"}>
                <input type="radio" name="project-delete-mode" value="files" checked={deleteMode === "files"} disabled={isDeleting} onChange={() => setDeleteMode("files")} />
                <span><strong>Удалить проект и файлы</strong><small>Выберите ниже, какие файлы удалить вместе с проектом.</small></span>
              </label>
            </div>

            {deleteMode === "files" && (
              <div className="delete-file-options">
                <label className="delete-file-option">
                  <input type="checkbox" checked={deleteSourceVideo} disabled={isDeleting} onChange={(event) => setDeleteSourceVideo(event.target.checked)} />
                  <span>Удалить исходное видео <small>{projectToDelete.video.file_name} · {formatSize(projectToDelete.video.file_size_bytes)}</small></span>
                </label>
                <label className="delete-file-option">
                  <input type="checkbox" checked={deleteExportedClips} disabled={isDeleting} onChange={(event) => setDeleteExportedClips(event.target.checked)} />
                  <span>Удалить вырезанные клипы <small>Только MP4-клипы этого проекта.</small></span>
                </label>
              </div>
            )}

            <p className="delete-note">Исходный файл субтитров не удаляется.</p>
            {deleteError && <p className="form-error" role="alert">{deleteError}</p>}

            <div className="dialog-actions">
              <button className="secondary-button" type="button" disabled={isDeleting} onClick={() => setProjectToDelete(null)}>Отмена</button>
              <button className="danger-button" type="button" disabled={isDeleting} onClick={() => void deleteProject()}>
                {isDeleting ? "Удаляем…" : "Удалить"}
              </button>
            </div>
          </section>
        </div>
      )}

      <AiSettingsDialog
        aiSettings={aiSettings}
        apiKeyInput={apiKeyInput}
        error={aiSettingsError}
        isOpen={isAiSettingsOpen}
        isSaving={isSavingAiSettings}
        onClose={() => !isSavingAiSettings && setAiSettingsOpen(false)}
        onDelete={() => void deleteOpenAiApiKey()}
        onInputChange={setApiKeyInput}
        onSave={() => void saveOpenAiApiKey()}
      />
    </main>
  );
}

type AiSettingsDialogProps = {
  aiSettings: AiSettings | null;
  apiKeyInput: string;
  error: string | null;
  isOpen: boolean;
  isSaving: boolean;
  onClose: () => void;
  onDelete: () => void;
  onInputChange: (value: string) => void;
  onSave: () => void;
};

function AiSettingsDialog({
  aiSettings,
  apiKeyInput,
  error,
  isOpen,
  isSaving,
  onClose,
  onDelete,
  onInputChange,
  onSave
}: AiSettingsDialogProps) {
  if (!isOpen) return null;
  const canDeleteKey = aiSettings?.key_status === "saved";

  return (
    <div className="backdrop" role="presentation" onMouseDown={onClose}>
      <section className="dialog settings-dialog" role="dialog" aria-modal="true" aria-labelledby="ai-settings-title" onMouseDown={(event) => event.stopPropagation()}>
        <div className="dialog-heading">
          <h2 id="ai-settings-title">Настройки</h2>
          <button className="icon-button" type="button" aria-label="Закрыть" disabled={isSaving} onClick={onClose}>×</button>
        </div>
        <div className="settings-layout">
          <nav className="settings-tabs" aria-label="Разделы настроек">
            <button className="settings-tab is-active" type="button" aria-current="page">AI Settings</button>
          </nav>
          <section className="settings-content" aria-label="AI Settings">
            <label className="api-key-field">
              OpenAI API key
              <span className="api-key-input-row">
                <input
                  autoFocus
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={canDeleteKey ? "Вставьте новый ключ, чтобы заменить" : "sk-…"}
                  value={apiKeyInput}
                  disabled={isSaving || aiSettings === null}
                  onChange={(event) => onInputChange(event.target.value)}
                />
                <button className="settings-key-action" type="button" title={canDeleteKey ? "Заменить ключ" : "Сохранить ключ"} aria-label={canDeleteKey ? "Заменить ключ" : "Сохранить ключ"} disabled={isSaving || !apiKeyInput.trim() || aiSettings === null} onClick={onSave}>
                  <CheckIcon />
                </button>
                {canDeleteKey && <button className="settings-key-action is-danger" type="button" title="Удалить ключ" aria-label="Удалить ключ" disabled={isSaving} onClick={onDelete}><TrashIcon /></button>}
              </span>
            </label>
            {error && <p className="form-error" role="alert">{error}</p>}
          </section>
        </div>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
