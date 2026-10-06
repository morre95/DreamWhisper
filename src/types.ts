export interface Settings {
  python_path: string; model_path: string; batch_size: number;
  auto_import: boolean; transcription_enabled: boolean;
  start_at_login: boolean;
}
export interface Device {
  id: string; block_path: string; vendor: string; model: string; serial: string;
  uuid: string; label: string; mount_path: string | null; registered: boolean;
}
export interface Recording {
  id: string; name: string; archive_path: string; sha256: string; size: number;
  source_modified_at: string | null; imported_at: string; status: string;
  error: string | null; attempts: number; duration: number | null;
}
export interface Segment {
  id: number; start: number; end: number; text: string; edited_text: string | null;
  words: { start: number; end: number; word: string; probability: number }[];
}
export interface Transcript {
  run_id: string | null; metadata: Record<string, unknown> | null; segments: Segment[];
}
export interface Run { id: string; created_at: string; metadata: Record<string, unknown> }
export interface ImportReport { imported: number; skipped: number; errors: string[] }
export interface Snapshot {
  recordings: Recording[]; devices: Device[]; settings: Settings; data_dir: string;
  activity: string; device_error: string | null; last_import: ImportReport | null;
}
