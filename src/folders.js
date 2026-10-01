import { documentDir, homeDir } from '@tauri-apps/api/path';

// a bare account has no documents folder registered
export async function documentsOrHomeDir() {
  try {
    return await documentDir();
  } catch {
    return await homeDir();
  }
}
