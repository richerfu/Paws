import { DefaultXComponent } from '@ohos-rs/ability';
import pawsUi from 'libpaws_ui.so';

@Entry
@Component
struct Index {
  onBackPress(): boolean | void {
    if (typeof pawsUi.onBackPressIntercept === 'function') {
      return pawsUi.onBackPressIntercept();
    }
    return false;
  }

  build() {
    RelativeContainer() {
      DefaultXComponent({ moduleName: 'paws_ui' })
        .height('100%')
        .width('100%')
        .alignRules({
          top: { anchor: '__container__', align: VerticalAlign.Top },
          bottom: { anchor: '__container__', align: VerticalAlign.Bottom },
          left: { anchor: '__container__', align: HorizontalAlign.Start },
          right: { anchor: '__container__', align: HorizontalAlign.End },
        })
        .id('pawsRoot')
    }
    .height('100%')
    .width('100%')
  }
}
