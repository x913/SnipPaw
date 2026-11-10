import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent, WheelEvent } from "react";
import { formatSubtitleTimecode, formatTimecode, parseTimecode } from "./timecode";
import type { AiAnalysis, AiMoment, AiSettings, AudioWaveform, ClipExport, Project, SubtitleCue } from "./types";

const MIN_CLIP_LENGTH_SECONDS = 0.1;
const TIMELINE_STEP_SECONDS = 0.1;
const SELECTION_TIMELINE_OCCUPANCY = 0.75;
const MIN_TIMELINE_WINDOW_SECONDS = MIN_CLIP_LENGTH_SECONDS / SELECTION_TIMELINE_OCCUPANCY;
const DEFAULT_TIMELINE_WINDOW_SECONDS = 20;
const subtitleFilters = [
  { name: "Субтитры", extensions: ["srt", "vtt"] }
];

type Boundary = "start" | "end";
type ExportQuality = "high" | "balanced" | "compact";
type IconName = "play" | "zoom-in" | "zoom-out" | "center" | "clip-start" | "clip-end" | "frame" | "quality" | "cut" | "magic" | "settings";
type TrackpadGestureEvent = Event & { clientX?: number; scale: number };

const qualityLabels: Record<ExportQuality, string> = {
  high: "Высокое качество",
  balanced: "Обычное качество",
  compact: "Компактный размер"
};

const aiPromptPresets = [
  {
    label: "Интересные",
    instruction: "Найди самостоятельные, понятные без контекста фрагменты: сильное мнение, неожиданный вывод, полезную мысль или эмоциональный момент."
  },
  {
    label: "Шутки",
    instruction: "Найди смешные, ироничные и неожиданные реплики, которые могут работать как короткий развлекательный фрагмент."
  },
  {
    label: "Конфликты",
    instruction: "Найди споры, резкие несогласия, эмоциональные столкновения и полярные мнения."
  },
  {
    label: "Сильные мысли",
    instruction: "Найди ясные тезисы, сильные выводы и содержательные формулировки, которые интересны сами по себе."
  },
  {
    label: "Спорные мнения",
    instruction: "Найди резкие, спорные или провокационные утверждения, которые могут вызвать обсуждение."
  },
  {
    label: "Советы",
    instruction: "Найди практические советы, конкретные рекомендации и полезные выводы для зрителя."
  }
];

type ProjectEditorProps = {
  project: Project;
  onBack: () => void;
  onOpenAiSettings: () => void;
  aiKeyStatusVersion: number;
  onProjectUpdated: (project: Project) => void;
};

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

function Icon({ name }: { name: IconName }) {
  if (name === "play") {
    return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7z" fill="currentColor" /></svg>;
  }
  if (name === "zoom-in" || name === "zoom-out") {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round">
        <circle cx="10.5" cy="10.5" r="5.5" />
        <path d="m15 15 4 4M7.5 10.5h6" />
        {name === "zoom-in" && <path d="M10.5 7.5v6" />}
      </svg>
    );
  }
  if (name === "center") {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
        <circle cx="12" cy="12" r="4" />
        <path d="M12 3v3M12 18v3M3 12h3M18 12h3" />
      </svg>
    );
  }
  if (name === "clip-start" || name === "clip-end") {
    const isStart = name === "clip-start";
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d={isStart ? "m16 5-7 7 7 7M6 5v14" : "m8 5 7 7-7 7M18 5v14"} />
      </svg>
    );
  }
  if (name === "frame") {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
        <rect x="4" y="6" width="16" height="12" rx="1.5" />
        <path d="M12 3v18M8 9h-1M8 15h-1M17 9h-1M17 15h-1" />
      </svg>
    );
  }
  if (name === "quality") {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.1 2.1-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.55V20.3h-3v-.1A1.7 1.7 0 0 0 10.7 18.65a1.7 1.7 0 0 0-1.88.34l-.06.06-2.1-2.1.06-.06A1.7 1.7 0 0 0 7.06 15a1.7 1.7 0 0 0-1.55-1.03h-.1v-3h.1A1.7 1.7 0 0 0 7.06 9.94a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.1-2.1.06.06a1.7 1.7 0 0 0 1.88.34 1.7 1.7 0 0 0 1.03-1.55v-.1h3v.1a1.7 1.7 0 0 0 1.03 1.55 1.7 1.7 0 0 0 1.88-.34L17.7 5.9l2.1 2.1-.06.06a1.7 1.7 0 0 0-.34 1.88 1.7 1.7 0 0 0 1.55 1.03h.1v3h-.1A1.7 1.7 0 0 0 19.4 15Z" />
      </svg>
    );
  }
  if (name === "magic") {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
        <path d="m14.5 3 .75 2.75L18 6.5l-2.75.75L14.5 10l-.75-2.75L11 6.5l2.75-.75zM7.5 10l1.25 4.25L13 15.5l-4.25 1.25L7.5 21l-1.25-4.25L2 15.5l4.25-1.25z" />
        <path d="m18.5 13 .5 1.5 1.5.5-1.5.5-.5 1.5-.5-1.5-1.5-.5 1.5-.5z" />
      </svg>
    );
  }
  if (name === "settings") {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.1 2.1-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.55v.1h-3v-.1a1.7 1.7 0 0 0-1.03-1.55 1.7 1.7 0 0 0-1.88.34l-.06.06-2.1-2.1.06-.06A1.7 1.7 0 0 0 7.06 15a1.7 1.7 0 0 0-1.55-1.03h-.1v-3h.1A1.7 1.7 0 0 0 7.06 9.94a1.7 1.7 0 0 0-.34-1.88L6.66 8l2.1-2.1.06.06a1.7 1.7 0 0 0 1.88.34 1.7 1.7 0 0 0 1.03-1.55v-.1h3v.1a1.7 1.7 0 0 0 1.03 1.55 1.7 1.7 0 0 0 1.88-.34l.06-.06 2.1 2.1-.06.06a1.7 1.7 0 0 0-.34 1.88 1.7 1.7 0 0 0 1.55 1.03h.1v3h-.1A1.7 1.7 0 0 0 19.4 15Z" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <circle cx="6" cy="6" r="2.5" />
      <circle cx="6" cy="18" r="2.5" />
      <path d="m8 7.5 10 9M8 16.5l10-9" />
    </svg>
  );
}

export function ProjectEditor({ project, onBack, onOpenAiSettings, aiKeyStatusVersion, onProjectUpdated }: ProjectEditorProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const previewEndTimeRef = useRef<number | null>(null);
  const waveformCanvasRef = useRef<HTMLCanvasElement>(null);
  const timelineWorkspaceRef = useRef<HTMLDivElement>(null);
  const gestureScaleRef = useRef<number | null>(null);
  const [videoSource, setVideoSource] = useState<string | null>(null);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [startTime, setStartTime] = useState(0);
  const [endTime, setEndTime] = useState(0);
  const [startInput, setStartInput] = useState("00:00:00");
  const [endInput, setEndInput] = useState("00:00:00");
  const [timelineWindow, setTimelineWindow] = useState(0);
  const [timelineStart, setTimelineStart] = useState(0);
  const [audioWaveform, setAudioWaveform] = useState<AudioWaveform | null>(null);
  const [isWaveformLoading, setWaveformLoading] = useState(false);
  const [isMuted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [isPreviewingSelection, setPreviewingSelection] = useState(false);
  const [isExporting, setExporting] = useState(false);
  const [exportQuality, setExportQuality] = useState<ExportQuality>("high");
  const [isQualityMenuOpen, setQualityMenuOpen] = useState(false);
  const [subtitles, setSubtitles] = useState(project.subtitles);
  const [activeSubtitleIndex, setActiveSubtitleIndex] = useState<number | null>(null);
  const [fileNameHint, setFileNameHint] = useState<string | null>(null);
  const [isImportingSubtitles, setImportingSubtitles] = useState(false);
  const [aiAnalysis, setAiAnalysis] = useState<AiAnalysis | null>(project.ai_analysis ?? null);
  const [isAiMenuOpen, setAiMenuOpen] = useState(false);
  const [isAiResultsOpen, setAiResultsOpen] = useState(false);
  const [aiInstruction, setAiInstruction] = useState(aiPromptPresets[0].instruction);
  const [hasOpenAiKey, setHasOpenAiKey] = useState<boolean | null>(null);
  const [isCheckingAiSettings, setCheckingAiSettings] = useState(false);
  const [isAnalyzingSubtitles, setAnalyzingSubtitles] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [exportedClip, setExportedClip] = useState<ClipExport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const selectedDuration = Math.max(0, endTime - startTime);
  const visibleWindow = duration > 0
    ? Math.min(Math.max(timelineWindow, Math.min(MIN_TIMELINE_WINDOW_SECONDS, duration)), duration)
    : 0;
  const maxTimelineStart = Math.max(0, duration - visibleWindow);
  const visibleStart = clamp(timelineStart, 0, maxTimelineStart);
  const visibleEnd = visibleStart + visibleWindow;
  const startHandleVisible = startTime >= visibleStart && startTime <= visibleEnd;
  const endHandleVisible = endTime >= visibleStart && endTime <= visibleEnd;
  const currentTimeVisible = currentTime >= visibleStart && currentTime <= visibleEnd;
  const selectedVisibleStart = clamp(startTime, visibleStart, visibleEnd);
  const selectedVisibleEnd = clamp(endTime, visibleStart, visibleEnd);
  const selectedLeftPercent = visibleWindow > 0 ? ((selectedVisibleStart - visibleStart) / visibleWindow) * 100 : 0;
  const selectedWidthPercent = visibleWindow > 0 ? Math.max(0, ((selectedVisibleEnd - selectedVisibleStart) / visibleWindow) * 100) : 0;
  const rulerTicks = useMemo(() => {
    if (visibleWindow <= 0) return [];
    return Array.from({ length: 5 }, (_, index) => visibleStart + (visibleWindow * index) / 4);
  }, [visibleStart, visibleWindow]);

  useEffect(() => {
    let cancelled = false;
    setVideoSource(null);
    void invoke<string>("preview_video_url", { projectId: project.id })
      .then((url) => {
        if (!cancelled) setVideoSource(url);
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(String(reason));
      });

    return () => {
      cancelled = true;
    };
  }, [project.id]);

  useEffect(() => {
    let cancelled = false;
    setAudioWaveform(null);
    setWaveformLoading(true);
    void invoke<AudioWaveform>("audio_waveform", { projectId: project.id })
      .then((waveform) => {
        if (!cancelled) setAudioWaveform(waveform);
      })
      .catch(() => {
        if (!cancelled) setAudioWaveform(null);
      })
      .finally(() => {
        if (!cancelled) setWaveformLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [project.id]);

  useEffect(() => {
    setSubtitles(project.subtitles);
    setAiAnalysis(project.ai_analysis ?? null);
    setActiveSubtitleIndex(null);
    setFileNameHint(null);
  }, [project.ai_analysis, project.id, project.subtitles]);

  useEffect(() => {
    const canvas = waveformCanvasRef.current;
    if (!canvas || !audioWaveform || audioWaveform.peaks.length === 0 || duration <= 0 || visibleWindow <= 0) return;

    const draw = () => {
      const width = Math.round(canvas.clientWidth);
      const height = Math.round(canvas.clientHeight);
      if (width <= 0 || height <= 0) return;

      const pixelRatio = window.devicePixelRatio || 1;
      canvas.width = Math.round(width * pixelRatio);
      canvas.height = Math.round(height * pixelRatio);
      const context = canvas.getContext("2d");
      if (!context) return;

      context.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
      context.clearRect(0, 0, width, height);

      const firstPeak = clamp(Math.floor(visibleStart * audioWaveform.peaks_per_second), 0, audioWaveform.peaks.length - 1);
      const lastPeak = clamp(Math.ceil(visibleEnd * audioWaveform.peaks_per_second), firstPeak + 1, audioWaveform.peaks.length);
      const peakRange = Math.max(1, lastPeak - firstPeak);
      const middle = height / 2;
      context.fillStyle = "rgba(193, 170, 255, 0.82)";

      for (let x = 0; x < width; x += 1) {
        const from = firstPeak + Math.floor((x * peakRange) / width);
        const to = Math.min(lastPeak, firstPeak + Math.ceil(((x + 1) * peakRange) / width));
        let peak = 0;
        for (let index = from; index < to; index += 1) {
          peak = Math.max(peak, audioWaveform.peaks[index] ?? 0);
        }
        const amplitude = Math.pow(peak / 32_768, 0.55);
        const barHeight = Math.max(1, middle * amplitude);
        context.fillRect(x, middle - barHeight, 1, barHeight * 2);
      }
    };

    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    draw();
    return () => observer.disconnect();
  }, [audioWaveform, duration, visibleEnd, visibleStart, visibleWindow]);

  function seek(seconds: number) {
    const video = videoRef.current;
    if (!video) return;
    const nextTime = clamp(seconds, 0, duration);
    video.currentTime = nextTime;
    setCurrentTime(nextTime);
    keepTimeVisible(nextTime);
  }

  function currentVideoTime() {
    const video = videoRef.current;
    if (video && Number.isFinite(video.currentTime)) {
      return clamp(video.currentTime, 0, duration);
    }

    return currentTime;
  }

  function setAudioVolume(nextVolume: number) {
    const video = videoRef.current;
    const normalizedVolume = clamp(nextVolume, 0, 1);
    if (!video) return;

    video.volume = normalizedVolume;
    video.muted = normalizedVolume === 0;
    setVolume(normalizedVolume);
    setMuted(video.muted);
  }

  function toggleMute() {
    const video = videoRef.current;
    if (!video) return;

    const nextMuted = !video.muted;
    video.muted = nextMuted;
    if (!nextMuted && video.volume === 0) video.volume = 1;
    setMuted(nextMuted);
    setVolume(video.volume);
  }

  function focusTimelineAt(time: number, nextWindow = visibleWindow) {
    if (duration <= 0) return;
    const window = Math.min(Math.max(nextWindow, Math.min(MIN_TIMELINE_WINDOW_SECONDS, duration)), duration);
    const maximumStart = Math.max(0, duration - window);
    setTimelineWindow(window);
    setTimelineStart(clamp(time - window / 2, 0, maximumStart));
  }

  function zoomTimelineAround(time: number, nextWindow: number) {
    if (duration <= 0 || visibleWindow <= 0) return;
    const window = Math.min(Math.max(nextWindow, Math.min(MIN_TIMELINE_WINDOW_SECONDS, duration)), duration);
    const relativePosition = clamp((time - visibleStart) / visibleWindow, 0, 1);
    const maximumStart = Math.max(0, duration - window);
    setTimelineWindow(window);
    setTimelineStart(clamp(time - window * relativePosition, 0, maximumStart));
  }

  function keepTimeVisible(time: number) {
    if (duration <= 0 || visibleWindow <= 0) return;
    setTimelineStart((currentStart) => {
      const maximumStart = Math.max(0, duration - visibleWindow);
      if (time < currentStart) return clamp(time - visibleWindow * 0.15, 0, maximumStart);
      if (time > currentStart + visibleWindow) return clamp(time - visibleWindow * 0.85, 0, maximumStart);
      return currentStart;
    });
  }

  function zoomTimeline(direction: "in" | "out") {
    const factor = direction === "in" ? 0.5 : 2;
    focusTimelineAt(currentVideoTime(), visibleWindow * factor);
  }

  function focusSelection() {
    focusTimelineAt((startTime + endTime) / 2, selectedDuration / SELECTION_TIMELINE_OCCUPANCY);
  }

  function focusFragment(start: number, end: number) {
    focusTimelineAt((start + end) / 2, (end - start) / SELECTION_TIMELINE_OCCUPANCY);
  }

  function seekFromTimeline(event: MouseEvent<HTMLDivElement>) {
    if (duration <= 0 || visibleWindow <= 0) return;
    if ((event.target as HTMLElement).closest("input")) return;

    const bounds = event.currentTarget.getBoundingClientRect();
    const position = clamp((event.clientX - bounds.left) / bounds.width, 0, 1);
    seek(visibleStart + visibleWindow * position);
  }

  function zoomTimelineWithTrackpad(event: WheelEvent<HTMLDivElement>) {
    if (!event.ctrlKey || duration <= 0 || visibleWindow <= 0) return;

    event.preventDefault();
    const bounds = event.currentTarget.getBoundingClientRect();
    const position = clamp((event.clientX - bounds.left) / bounds.width, 0, 1);
    const focusTime = visibleStart + visibleWindow * position;
    const zoomFactor = clamp(Math.exp(event.deltaY * 0.01), 0.5, 2);
    zoomTimelineAround(focusTime, visibleWindow * zoomFactor);
  }

  useEffect(() => {
    const workspace = timelineWorkspaceRef.current;
    if (!workspace) return;

    const startGesture = (event: Event) => {
      const gesture = event as TrackpadGestureEvent;
      gestureScaleRef.current = gesture.scale;
      event.preventDefault();
    };
    const changeGesture = (event: Event) => {
      const gesture = event as TrackpadGestureEvent;
      if (duration <= 0 || visibleWindow <= 0) return;

      event.preventDefault();
      const previousScale = gestureScaleRef.current ?? gesture.scale;
      const zoomFactor = clamp(previousScale / gesture.scale, 0.5, 2);
      gestureScaleRef.current = gesture.scale;
      const bounds = workspace.getBoundingClientRect();
      const position = clamp(((gesture.clientX ?? (bounds.left + bounds.width / 2)) - bounds.left) / bounds.width, 0, 1);
      zoomTimelineAround(visibleStart + visibleWindow * position, visibleWindow * zoomFactor);
    };
    const endGesture = () => {
      gestureScaleRef.current = null;
    };

    workspace.addEventListener("gesturestart", startGesture, { passive: false });
    workspace.addEventListener("gesturechange", changeGesture, { passive: false });
    workspace.addEventListener("gestureend", endGesture);
    return () => {
      workspace.removeEventListener("gesturestart", startGesture);
      workspace.removeEventListener("gesturechange", changeGesture);
      workspace.removeEventListener("gestureend", endGesture);
    };
  }, [duration, visibleStart, visibleWindow]);

  function setBoundary(boundary: Boundary, rawValue: number, shouldSeek = true) {
    if (duration <= 0) return;
    const nextValue = boundary === "start"
      ? clamp(rawValue, 0, Math.max(0, endTime - MIN_CLIP_LENGTH_SECONDS))
      : clamp(rawValue, Math.min(duration, startTime + MIN_CLIP_LENGTH_SECONDS), duration);

    setError(null);
    setExportedClip(null);
    if (boundary === "start") {
      setStartTime(nextValue);
      setStartInput(formatTimecode(nextValue));
    } else {
      setEndTime(nextValue);
      setEndInput(formatTimecode(nextValue));
    }
    keepTimeVisible(nextValue);
    if (shouldSeek) seek(nextValue);
  }

  function setBoundaryAtCurrentFrame(boundary: Boundary) {
    const frameTime = currentVideoTime();
    let nextStart = startTime;
    let nextEnd = endTime;

    if (boundary === "start") {
      nextStart = frameTime;
      if (nextEnd - nextStart < MIN_CLIP_LENGTH_SECONDS) {
        nextEnd = Math.min(duration, nextStart + MIN_CLIP_LENGTH_SECONDS);
        if (nextEnd - nextStart < MIN_CLIP_LENGTH_SECONDS) {
          nextStart = Math.max(0, nextEnd - MIN_CLIP_LENGTH_SECONDS);
        }
      }
    } else {
      nextEnd = frameTime;
      if (nextEnd - nextStart < MIN_CLIP_LENGTH_SECONDS) {
        nextStart = Math.max(0, nextEnd - MIN_CLIP_LENGTH_SECONDS);
        if (nextEnd - nextStart < MIN_CLIP_LENGTH_SECONDS) {
          nextEnd = Math.min(duration, nextStart + MIN_CLIP_LENGTH_SECONDS);
        }
      }
    }

    setError(null);
    setExportedClip(null);
    setStartTime(nextStart);
    setEndTime(nextEnd);
    setStartInput(formatTimecode(nextStart));
    setEndInput(formatTimecode(nextEnd));
    keepTimeVisible(frameTime);
  }

  function selectSubtitle(cue: SubtitleCue, index: number, shouldPreview = false) {
    if (duration <= 0) return;

    const nextStart = clamp(cue.start_seconds, 0, duration);
    const nextEnd = clamp(cue.end_seconds, 0, duration);
    if (nextEnd <= nextStart) {
      setError("Интервал реплики находится за пределами видео.");
      return;
    }

    setError(null);
    setExportedClip(null);
    previewEndTimeRef.current = null;
    setPreviewingSelection(false);
    videoRef.current?.pause();
    setActiveSubtitleIndex(index);
    setFileNameHint(cue.text);
    setStartTime(nextStart);
    setEndTime(nextEnd);
    setStartInput(formatTimecode(nextStart));
    setEndInput(formatTimecode(nextEnd));
    seek(nextStart);
    focusFragment(nextStart, nextEnd);
    if (shouldPreview) previewFragment(nextStart, nextEnd);
  }

  async function importSubtitles() {
    setError(null);
    try {
      const selected = await open({ multiple: false, directory: false, filters: subtitleFilters });
      if (!selected || Array.isArray(selected)) return;

      setImportingSubtitles(true);
      const updatedProject = await invoke<Project>("import_subtitles", {
        projectId: project.id,
        path: selected
      });
      setSubtitles(updatedProject.subtitles);
      setAiAnalysis(null);
      setAiResultsOpen(false);
      setActiveSubtitleIndex(null);
      setFileNameHint(null);
      onProjectUpdated(updatedProject);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setImportingSubtitles(false);
    }
  }

  function toggleAiMenu() {
    setAiMenuOpen((isOpen) => !isOpen);
    setAiError(null);
  }

  useEffect(() => {
    if (!isAiMenuOpen) return;

    let cancelled = false;
    setCheckingAiSettings(true);
    void invoke<AiSettings>("get_ai_settings")
      .then((settings) => {
        if (!cancelled) setHasOpenAiKey(settings.key_status !== "not_configured");
      })
      .catch((reason) => {
        if (!cancelled) setAiError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setCheckingAiSettings(false);
      });

    return () => {
      cancelled = true;
    };
  }, [aiKeyStatusVersion, isAiMenuOpen]);

  async function analyzeSubtitles() {
    if (!aiInstruction.trim() || subtitles.length === 0 || isAnalyzingSubtitles) return;

    setAnalyzingSubtitles(true);
    setAiError(null);
    try {
      const updatedProject = await invoke<Project>("analyze_subtitles", {
        projectId: project.id,
        instruction: aiInstruction
      });
      const updatedAnalysis = updatedProject.ai_analysis ?? null;
      setAiAnalysis(updatedAnalysis);
      setAiMenuOpen(false);
      setAiResultsOpen(updatedAnalysis !== null);
      onProjectUpdated(updatedProject);
    } catch (reason) {
      setAiError(String(reason));
    } finally {
      setAnalyzingSubtitles(false);
    }
  }

  function openAiSettings() {
    setAiMenuOpen(false);
    setAiResultsOpen(false);
    onOpenAiSettings();
  }

  function selectAiMoment(moment: AiMoment, shouldPreview = false) {
    const cue = subtitles[moment.start_cue_index];
    if (!cue) {
      setError("Исходная реплика для результата ИИ не найдена.");
      return;
    }

    selectSubtitle(cue, moment.start_cue_index, shouldPreview);
  }

  function applyManualTime(boundary: Boundary) {
    const value = boundary === "start" ? startInput : endInput;
    const parsed = parseTimecode(value);
    if (parsed === null || parsed > duration) {
      setError(`Введите таймкод от 00:00:00 до ${formatTimecode(duration)}.`);
      return;
    }

    setBoundary(boundary, parsed);
  }

  function previewFragment(rawStart: number, rawEnd: number) {
    const video = videoRef.current;
    if (!video || duration <= 0) return;

    const nextStart = clamp(rawStart, 0, duration);
    const nextEnd = clamp(rawEnd, nextStart + MIN_CLIP_LENGTH_SECONDS, duration);
    if (nextEnd <= nextStart) return;

    setError(null);
    video.pause();
    previewEndTimeRef.current = nextEnd;
    setPreviewingSelection(true);
    video.currentTime = nextStart;
    setCurrentTime(nextStart);
    keepTimeVisible(nextStart);
    void video.play().catch(() => {
      previewEndTimeRef.current = null;
      setPreviewingSelection(false);
      setError("Не удалось запустить воспроизведение видео.");
    });
  }

  function previewSelection() {
    previewFragment(startTime, endTime);
  }

  useEffect(() => {
    const handleSpacebar = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat || (event.key !== " " && event.code !== "Space")) return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.target instanceof HTMLElement && event.target.closest("input, textarea, select, [contenteditable='true']")) return;

      const video = videoRef.current;
      if (!video || duration <= 0) return;

      event.preventDefault();
      if (previewEndTimeRef.current !== null) {
        video.pause();
        previewEndTimeRef.current = null;
        setPreviewingSelection(false);
        return;
      }

      previewSelection();
    };

    window.addEventListener("keydown", handleSpacebar);
    return () => window.removeEventListener("keydown", handleSpacebar);
  }, [duration, endTime, startTime]);

  function handleTimeUpdate() {
    const video = videoRef.current;
    if (!video) return;

    const nextCurrentTime = video.currentTime;
    setCurrentTime(nextCurrentTime);
    keepTimeVisible(nextCurrentTime);
    const previewEndTime = previewEndTimeRef.current;
    if (previewEndTime !== null && nextCurrentTime >= previewEndTime) {
      video.pause();
      seek(previewEndTime);
      previewEndTimeRef.current = null;
      setPreviewingSelection(false);
    }
  }

  function handleMetadataLoaded() {
    const video = videoRef.current;
    if (!video || !Number.isFinite(video.duration) || video.duration <= 0) {
      setError("Не удалось определить длительность видео.");
      return;
    }

    setDuration(video.duration);
    video.defaultMuted = false;
    video.muted = false;
    video.volume = 1;
    setMuted(false);
    setVolume(1);
    setCurrentTime(0);
    setStartTime(0);
    setEndTime(Math.min(video.duration, 10));
    setStartInput(formatTimecode(0));
    setEndInput(formatTimecode(Math.min(video.duration, 10)));
    setTimelineWindow(Math.min(video.duration, DEFAULT_TIMELINE_WINDOW_SECONDS));
    setTimelineStart(0);
    setError(null);
  }

  async function exportClip() {
    if (duration <= 0 || selectedDuration < MIN_CLIP_LENGTH_SECONDS) return;

    setExporting(true);
    setError(null);
    setExportedClip(null);
    try {
      const clip = await invoke<ClipExport>("export_clip", {
        projectId: project.id,
        startSeconds: startTime,
        endSeconds: endTime,
        fileNameHint,
        quality: exportQuality
      });
      setExportedClip(clip);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setExporting(false);
    }
  }

  return (
    <main className="editor-shell">
      <header className="editor-topbar">
        <button className="back-button" type="button" onClick={onBack}>‹ Все проекты</button>
        <div className="editor-title">
          <span>{project.name}</span>
          <small>{project.video.file_name}</small>
        </div>
        <button className="editor-settings-button" type="button" title="Настройки" aria-label="Настройки" onClick={openAiSettings}><Icon name="settings" /></button>
      </header>

      <div className="editor-workspace">
        <section className="preview-area">
          <div className="video-frame">
            <video
              ref={videoRef}
              controls
              playsInline
              preload="auto"
              src={videoSource ?? undefined}
              onLoadedMetadata={handleMetadataLoaded}
              onTimeUpdate={handleTimeUpdate}
              onVolumeChange={(event) => {
                setMuted(event.currentTarget.muted);
                setVolume(event.currentTarget.volume);
              }}
              onPause={() => {
                if (previewEndTimeRef.current !== null) {
                  previewEndTimeRef.current = null;
                  setPreviewingSelection(false);
                }
              }}
              onEnded={() => {
                previewEndTimeRef.current = null;
                setPreviewingSelection(false);
              }}
              onError={() => setError("Видео не удалось открыть. Для предпросмотра используйте MP4 или MOV с кодеком H.264.")}
            />
          </div>
          <div className="preview-status-row">
            <div className="playhead-time">{formatTimecode(currentTime)} / {duration > 0 ? formatTimecode(duration) : "—"}</div>
            <div className="audio-controls">
              <button className="audio-toggle" type="button" onClick={toggleMute}>{isMuted || volume === 0 ? "🔇 Включить звук" : "🔊 Звук"}</button>
              <input
                aria-label="Громкость предпросмотра"
                className="volume-slider"
                type="range"
                min="0"
                max="1"
                step="0.05"
                value={volume}
                onChange={(event) => setAudioVolume(Number(event.target.value))}
              />
            </div>
          </div>
        </section>

        <aside className="subtitle-panel" aria-label="Субтитры">
          <div className="subtitle-panel-heading">
            <h2>Субтитры</h2>
            <div className="subtitle-panel-actions">
              <button className={isAiMenuOpen ? "subtitle-icon-button is-active" : "subtitle-icon-button"} type="button" title="ИИ-анализ субтитров" aria-label="ИИ-анализ субтитров" aria-expanded={isAiMenuOpen} disabled={subtitles.length === 0} onClick={() => void toggleAiMenu()}><Icon name="magic" /></button>
              <button className="text-button" type="button" disabled={isImportingSubtitles} onClick={() => void importSubtitles()}>
                {isImportingSubtitles ? "Загружаем…" : subtitles.length > 0 ? "Заменить" : "Загрузить"}
              </button>
            </div>
          </div>

          {subtitles.length === 0 ? (
            <p className="subtitle-empty">Загрузите файл SRT или VTT.</p>
          ) : (
            <div className="subtitle-table-wrap">
              <table className="subtitle-table">
                <thead>
                  <tr><th scope="col">Интервал</th><th scope="col">Фраза</th></tr>
                </thead>
                <tbody>
                  {subtitles.map((cue, index) => (
                    <tr
                      key={`${cue.start_seconds}-${cue.end_seconds}-${index}`}
                      className={activeSubtitleIndex === index ? "subtitle-row subtitle-row-active" : "subtitle-row"}
                      role="button"
                      tabIndex={duration > 0 ? 0 : -1}
                      onClick={() => selectSubtitle(cue, index)}
                      onDoubleClick={() => selectSubtitle(cue, index, true)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          selectSubtitle(cue, index);
                        }
                      }}
                    >
                      <td>{formatSubtitleTimecode(cue.start_seconds)} — {formatSubtitleTimecode(cue.end_seconds)}</td>
                      <td>{cue.text}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          {isAiMenuOpen && (
            <section className="ai-menu-popover" role="dialog" aria-label="ИИ-анализ субтитров">
              <div className="ai-popover-heading">
                <strong>ИИ-анализ</strong>
                <button className="icon-button" type="button" aria-label="Закрыть меню ИИ" onClick={() => setAiMenuOpen(false)}>×</button>
              </div>
              <p className="ai-popover-copy">Выберите критерий — он подставится в инструкцию. Видео и звук не отправляются: анализируется только текст субтитров.</p>
              <div className="ai-preset-list" aria-label="Критерии поиска">
                {aiPromptPresets.map((preset) => (
                  <button key={preset.label} className={aiInstruction === preset.instruction ? "ai-preset is-selected" : "ai-preset"} type="button" onClick={() => setAiInstruction(preset.instruction)}>{preset.label}</button>
                ))}
              </div>
              <label className="ai-instruction-field">
                Инструкция
                <textarea value={aiInstruction} maxLength={800} disabled={isAnalyzingSubtitles} onChange={(event) => setAiInstruction(event.target.value)} />
              </label>
              {aiAnalysis && <button className="text-button ai-show-results-button" type="button" onClick={() => {
                setAiMenuOpen(false);
                setAiResultsOpen(true);
              }}>Показать сохранённый разбор</button>}
              {hasOpenAiKey === false && <p className="ai-key-warning">Добавьте API key в AI Settings.</p>}
              {aiError && <p className="form-error" role="alert">{aiError}</p>}
              <div className="ai-menu-actions">
                <button className="subtitle-icon-button" type="button" title="AI Settings" aria-label="AI Settings" onClick={openAiSettings}><Icon name="settings" /></button>
                <button className="primary-button" type="button" disabled={isCheckingAiSettings || hasOpenAiKey !== true || isAnalyzingSubtitles || !aiInstruction.trim()} onClick={() => void analyzeSubtitles()}>
                  {isAnalyzingSubtitles ? "Анализируем…" : aiAnalysis ? "Запустить заново" : "Найти моменты"}
                </button>
              </div>
            </section>
          )}

          {isAiResultsOpen && aiAnalysis && (
            <section className="ai-results-popover" role="dialog" aria-label="Результаты ИИ-анализа">
              <div className="ai-popover-heading">
                <div><strong>ИИ: интересные моменты</strong><span>{aiAnalysis.moments.length}</span></div>
                <button className="icon-button" type="button" aria-label="Закрыть результаты ИИ" onClick={() => setAiResultsOpen(false)}>×</button>
              </div>
              <p className="ai-results-instruction">{aiAnalysis.instruction}</p>
              {aiAnalysis.moments.length === 0 ? (
                <p className="subtitle-empty">ИИ не нашёл моментов по этому критерию.</p>
              ) : (
                <div className="ai-moment-list">
                  {aiAnalysis.moments.map((moment, index) => (
                    <button className="ai-moment-row" type="button" key={`${moment.start_cue_index}-${moment.end_cue_index}-${index}`} onClick={() => selectAiMoment(moment)} onDoubleClick={() => selectAiMoment(moment, true)}>
                      <span>{formatSubtitleTimecode(moment.start_seconds)}</span>
                      <strong>{moment.note}</strong>
                    </button>
                  ))}
                </div>
              )}
            </section>
          )}
        </aside>
      </div>

      <section className="trim-panel" aria-label="Границы будущего клипа">
        <div className="trim-toolbar">
          <strong className="selected-duration">{formatTimecode(selectedDuration)}</strong>
          <div className="timeline-toolbar-actions">
            <button className={isPreviewingSelection ? "icon-control-button is-active" : "icon-control-button"} type="button" title="Просмотреть выделенный фрагмент" aria-label="Просмотреть выделенный фрагмент" disabled={duration <= 0} onClick={previewSelection}>
              <Icon name="play" />
            </button>
            <button className="icon-control-button" type="button" title="Отдалить шкалу" aria-label="Отдалить шкалу: показать вдвое больше времени" disabled={duration <= 0 || visibleWindow >= duration} onClick={() => zoomTimeline("out")}>
              <Icon name="zoom-out" />
            </button>
            <button className="icon-control-button" type="button" title="Приблизить шкалу" aria-label="Приблизить шкалу: показать вдвое меньше времени" disabled={duration <= 0 || visibleWindow <= Math.min(MIN_TIMELINE_WINDOW_SECONDS, duration)} onClick={() => zoomTimeline("in")}>
              <Icon name="zoom-in" />
            </button>
            <button className="icon-control-button" type="button" title="Центрировать выделение на шкале" aria-label="Центрировать выделение на шкале" disabled={duration <= 0} onClick={focusSelection}>
              <Icon name="center" />
            </button>
          </div>
        </div>

        <div className="trim-workspace">
          <div className="timeline-workspace" ref={timelineWorkspaceRef} onWheel={zoomTimelineWithTrackpad}>
            <div className="timeline-labels" aria-hidden="true">
              {rulerTicks.map((time) => <span key={time}>{formatTimecode(time)}</span>)}
            </div>
            <div className="timeline-control" aria-label="Шкала времени: кликните, чтобы перейти к кадру" onClick={seekFromTimeline}>
              <div className="timeline-track" />
              <div className="timeline-selection" style={{ left: `${selectedLeftPercent}%`, width: `${selectedWidthPercent}%` }} />
              {currentTimeVisible && <div className="timeline-playhead" style={{ left: `${((currentTime - visibleStart) / visibleWindow) * 100}%` }} />}
              {startHandleVisible && (
                <input
                  aria-label="Начало клипа на увеличенной шкале"
                  className="timeline-range timeline-range-start"
                  type="range"
                  min={visibleStart}
                  max={visibleEnd}
                  step={TIMELINE_STEP_SECONDS}
                  value={startTime}
                  disabled={duration <= 0}
                  onChange={(event) => setBoundary("start", Number(event.target.value))}
                />
              )}
              {endHandleVisible && (
                <input
                  aria-label="Конец клипа на увеличенной шкале"
                  className="timeline-range timeline-range-end"
                  type="range"
                  min={visibleStart}
                  max={visibleEnd}
                  step={TIMELINE_STEP_SECONDS}
                  value={endTime}
                  disabled={duration <= 0}
                  onChange={(event) => setBoundary("end", Number(event.target.value))}
                />
              )}
            </div>
            {(isWaveformLoading || audioWaveform) && (
              <div className="audio-waveform" aria-label="Аудиодорожка: низкие столбцы обозначают тишину" onClick={seekFromTimeline}>
                <div className="audio-waveform-selection" style={{ left: `${selectedLeftPercent}%`, width: `${selectedWidthPercent}%` }} />
                {audioWaveform ? <canvas ref={waveformCanvasRef} /> : <span className="audio-waveform-loading">Готовим аудиодорожку…</span>}
                {currentTimeVisible && <div className="audio-waveform-playhead" style={{ left: `${((currentTime - visibleStart) / visibleWindow) * 100}%` }} />}
              </div>
            )}
          </div>

          <div className="boundary-grid">
            <section className="boundary-card">
              <span className="boundary-heading boundary-start" title="Начало клипа" role="img" aria-label="Начало клипа"><Icon name="clip-start" /></span>
              <div className="time-input-row">
                <input
                  aria-label="Таймкод начала"
                  className="time-input"
                  value={startInput}
                  onChange={(event) => setStartInput(event.target.value)}
                  onBlur={() => applyManualTime("start")}
                  onKeyDown={(event) => event.key === "Enter" && event.currentTarget.blur()}
                />
                <div className="step-button-group" aria-label="Сдвиг начала на одну секунду">
                  <button type="button" title="Сдвинуть начало на 1 секунду назад" aria-label="Сдвинуть начало на 1 секунду назад" className="step-button" disabled={duration <= 0 || startTime <= 0} onClick={() => setBoundary("start", startTime - 1)}>−</button>
                  <button type="button" title="Сдвинуть начало на 1 секунду вперёд" aria-label="Сдвинуть начало на 1 секунду вперёд" className="step-button" disabled={duration <= 0 || startTime >= endTime - MIN_CLIP_LENGTH_SECONDS} onClick={() => setBoundary("start", startTime + 1)}>+</button>
                </div>
                <button type="button" className="current-time-button" title="Установить начало по текущему кадру видео" aria-label="Установить начало по текущему кадру видео" disabled={duration <= 0} onClick={() => setBoundaryAtCurrentFrame("start")}><Icon name="frame" /></button>
              </div>
            </section>

            <section className="boundary-card">
              <span className="boundary-heading boundary-end" title="Конец клипа" role="img" aria-label="Конец клипа"><Icon name="clip-end" /></span>
              <div className="time-input-row">
                <input
                  aria-label="Таймкод конца"
                  className="time-input"
                  value={endInput}
                  onChange={(event) => setEndInput(event.target.value)}
                  onBlur={() => applyManualTime("end")}
                  onKeyDown={(event) => event.key === "Enter" && event.currentTarget.blur()}
                />
                <div className="step-button-group" aria-label="Сдвиг конца на одну секунду">
                  <button type="button" title="Сдвинуть конец на 1 секунду назад" aria-label="Сдвинуть конец на 1 секунду назад" className="step-button" disabled={duration <= 0 || endTime <= startTime + MIN_CLIP_LENGTH_SECONDS} onClick={() => setBoundary("end", endTime - 1)}>−</button>
                  <button type="button" title="Сдвинуть конец на 1 секунду вперёд" aria-label="Сдвинуть конец на 1 секунду вперёд" className="step-button" disabled={duration <= 0 || endTime >= duration} onClick={() => setBoundary("end", endTime + 1)}>+</button>
                </div>
                <button type="button" className="current-time-button" title="Установить конец по текущему кадру видео" aria-label="Установить конец по текущему кадру видео" disabled={duration <= 0} onClick={() => setBoundaryAtCurrentFrame("end")}><Icon name="frame" /></button>
              </div>
            </section>
          </div>
        </div>

        {error && <p className="editor-error" role="alert">{error}</p>}
        {exportedClip && <p className="export-success">Готово: <strong>{exportedClip.file_name}</strong><span>{exportedClip.output_path}</span></p>}

        <div className="export-row">
          <div className="quality-menu">
            <button className={isQualityMenuOpen ? "icon-control-button is-active" : "icon-control-button"} type="button" title={`Качество: ${qualityLabels[exportQuality]}`} aria-label={`Настроить качество: ${qualityLabels[exportQuality]}`} aria-expanded={isQualityMenuOpen} disabled={isExporting} onClick={() => setQualityMenuOpen((current) => !current)}>
              <Icon name="quality" />
            </button>
            {isQualityMenuOpen && (
              <div className="quality-menu-popover" role="menu" aria-label="Качество сохранения">
                {(Object.keys(qualityLabels) as ExportQuality[]).map((quality) => (
                  <button key={quality} type="button" role="menuitemradio" aria-checked={exportQuality === quality} className={exportQuality === quality ? "quality-menu-option is-selected" : "quality-menu-option"} onClick={() => {
                    setExportQuality(quality);
                    setQualityMenuOpen(false);
                  }}>
                    {qualityLabels[quality]}
                  </button>
                ))}
              </div>
            )}
          </div>
          <button className="export-button" type="button" title={isExporting ? "Вырезаем…" : "Вырезать видео"} aria-label={isExporting ? "Вырезаем видео" : "Вырезать видео"} disabled={duration <= 0 || isExporting} onClick={() => void exportClip()}>
            {isExporting ? "…" : <Icon name="cut" />}
          </button>
        </div>
      </section>
    </main>
  );
}
