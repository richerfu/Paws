import { hilog } from '@kit.PerformanceAnalysisKit';
import { BackupExtensionAbility, BundleVersion } from '@kit.CoreFileKit';

const DOMAIN = 0x0000;
const LOG_TAG = 'PawsBackup';
const PAWS_BACKUP_ROOT = '/data/storage/el2/base/files/paws';

export default class EntryBackupAbility extends BackupExtensionAbility {
  async onBackup() {
    hilog.info(DOMAIN, LOG_TAG, 'backup started for %{public}s', PAWS_BACKUP_ROOT);
    await Promise.resolve();
  }

  async onRestore(bundleVersion: BundleVersion) {
    hilog.info(
      DOMAIN,
      LOG_TAG,
      'restore completed for %{public}s from %{public}s',
      PAWS_BACKUP_ROOT,
      JSON.stringify(bundleVersion),
    );
    await Promise.resolve();
  }
}
