import { FormExtensionAbility, formBindingData, formProvider } from '@kit.FormKit';
import { Want } from '@kit.AbilityKit';
import { fileIo } from '@kit.CoreFileKit';

export default class TinyPawsFormAbility extends FormExtensionAbility {
  private idsPath(): string { return `${this.context.filesDir}/tinypaws-form-ids.json`; }
  private ids(): string[] {
    try {
      if (fileIo.statSync(this.idsPath()).size > 16384) { return []; }
      return JSON.parse(fileIo.readTextSync(this.idsPath())) as string[];
    } catch (_) { return []; }
  }
  private save(ids: string[]): void {
    const file = fileIo.openSync(this.idsPath(), fileIo.OpenMode.WRITE_ONLY | fileIo.OpenMode.CREATE | fileIo.OpenMode.TRUNC);
    try { fileIo.writeSync(file.fd, JSON.stringify(ids.slice(-32))); } finally { fileIo.closeSync(file); }
  }
  onAddForm(want: Want): formBindingData.FormBindingData {
    const id = want.parameters?.['ohos.extra.param.key.form_identity'] as string;
    if (id) { const ids = this.ids().filter((item: string): boolean => item !== id); ids.push(id); this.save(ids); }
    return formBindingData.createFormBindingData({ status: '打开应用查看 VPN 状态', updated: '状态以应用为准' });
  }
  onUpdateForm(formId: string): void {
    formProvider.updateForm(formId, formBindingData.createFormBindingData({ status: '打开应用查看 VPN 状态', updated: '状态以应用为准' })).catch(() => {});
  }
  onRemoveForm(formId: string): void { this.save(this.ids().filter((id: string): boolean => id !== formId)); }
}
