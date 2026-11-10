export type VideoFile = {
  path: string;
  file_name: string;
  file_size_bytes: number;
};

export type SubtitleCue = {
  start_seconds: number;
  end_seconds: number;
  text: string;
};

export type AiMoment = {
  start_cue_index: number;
  end_cue_index: number;
  start_seconds: number;
  end_seconds: number;
  note: string;
};

export type AiAnalysis = {
  instruction: string;
  model: string;
  created_at_unix_ms: number;
  subtitle_fingerprint: string;
  moments: AiMoment[];
};

export type AiSettings = {
  key_status: "saved" | "unknown" | "not_configured";
  model: string;
};

export type Project = {
  id: string;
  name: string;
  video: VideoFile;
  subtitles: SubtitleCue[];
  ai_analysis?: AiAnalysis;
  workspace_path?: string;
  created_at_unix_ms: number;
};

export type ProjectDraft = {
  id: string;
  name: string;
};

export type ImportToolsStatus = {
  yt_dlp_available: boolean;
  ffmpeg_available: boolean;
  deno_available: boolean;
};

export type YoutubeDownloadProgress = {
  draft_id: string;
  phase: "starting" | "downloading" | "merging";
  downloaded_bytes: number | null;
  total_bytes: number | null;
  speed_bytes_per_second: number | null;
  eta_seconds: number | null;
};

export type YoutubeVideoInfo = {
  title: string;
};

export type ClipExport = {
  output_path: string;
  file_name: string;
  duration_seconds: number;
};

export type AudioWaveform = {
  peaks_per_second: number;
  peaks: number[];
};

export type ProjectDeletion = {
  source_video_deleted: boolean;
  exported_clips_deleted: number;
};
