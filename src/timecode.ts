export function formatTimecode(seconds: number) {
  const totalTenths = Math.max(0, Math.round(seconds * 10));
  const hours = Math.floor(totalTenths / 36_000);
  const minutes = Math.floor((totalTenths % 36_000) / 600);
  const wholeSeconds = Math.floor((totalTenths % 600) / 10);
  const tenths = totalTenths % 10;
  const fraction = tenths === 0 ? "" : `.${tenths}`;

  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(wholeSeconds).padStart(2, "0")}${fraction}`;
}

export function formatSubtitleTimecode(seconds: number) {
  const totalMilliseconds = Math.max(0, Math.round(seconds * 1000));
  const hours = Math.floor(totalMilliseconds / 3_600_000);
  const minutes = Math.floor((totalMilliseconds % 3_600_000) / 60_000);
  const wholeSeconds = Math.floor((totalMilliseconds % 60_000) / 1000);
  const milliseconds = totalMilliseconds % 1000;
  const base = `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(wholeSeconds).padStart(2, "0")}`;

  return milliseconds === 0 ? base : `${base}.${String(milliseconds).padStart(3, "0")}`;
}

export function parseTimecode(value: string) {
  const parts = value.trim().replace(",", ".").split(":");
  if (parts.length === 0 || parts.length > 3 || parts.some((part) => part === "")) return null;

  const values = parts.map(Number);
  if (values.some((part) => !Number.isFinite(part) || part < 0)) return null;

  const seconds = values[values.length - 1];
  const minutes = parts.length >= 2 ? values[values.length - 2] : 0;
  const hours = parts.length === 3 ? values[0] : 0;
  if (seconds >= 60 || minutes >= 60) return null;

  return hours * 3600 + minutes * 60 + seconds;
}
