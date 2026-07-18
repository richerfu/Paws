import { DefaultXComponent } from '@ohos-rs/ability';
import hmetaUi from 'libhmeta_ui.so';

@Entry
@Component
struct Index {
  onBackPress(): boolean | void {
    if (typeof hmetaUi.onBackPressIntercept === 'function') {
      return hmetaUi.onBackPressIntercept();
    }
    return false;
  }

  build() {
    RelativeContainer() {
      DefaultXComponent({ moduleName: 'hmeta_ui' })
        .height('100%')
        .width('100%')
        .alignRules({
          top: { anchor: '__container__', align: VerticalAlign.Top },
          bottom: { anchor: '__container__', align: VerticalAlign.Bottom },
          left: { anchor: '__container__', align: HorizontalAlign.Start },
          right: { anchor: '__container__', align: HorizontalAlign.End },
        })
        .id('hmetaRoot')
    }
    .height('100%')
    .width('100%')
  }
}
