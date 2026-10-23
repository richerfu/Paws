import { FormExtensionAbility, formBindingData } from '@kit.FormKit';
import { Want } from '@kit.AbilityKit';

// Keep the declared extension identity while refusing personal widget changes.
export default class TinyPawsFormAbility extends FormExtensionAbility {
  onAddForm(_want: Want): formBindingData.FormBindingData {
    throw new Error('Engine parity stage does not create or replace widgets');
  }
  onUpdateForm(_formId: string): void {}
  onRemoveForm(_formId: string): void {}
}
