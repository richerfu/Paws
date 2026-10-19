import { formBindingData, formProvider } from '@kit.FormKit';
import { fileIo } from '@kit.CoreFileKit';

export function updateTinyPawsForms(filesDir: string, status: string): void {
  try {
    const path = `${filesDir}/tinypaws-form-ids.json`;
    if (fileIo.statSync(path).size > 16384) { return; }
    const ids = JSON.parse(fileIo.readTextSync(path)) as string[];
    const data = formBindingData.createFormBindingData({ status, updated: `最近更新 ${new Date().toLocaleTimeString()}` });
    ids.slice(-32).forEach((id: string): void => { formProvider.updateForm(id, data).catch(() => {}); });
  } catch (_) {}
}
