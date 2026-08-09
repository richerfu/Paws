import { DefaultXComponent } from '@ohos-rs/ability';
import pawsUi from 'libpaws_ui.so';
import { getSafeAreaInsets, SafeAreaInsets } from '../common/SafeAreaBootstrap';

@Entry
@Component
struct Index {
  private readonly safeArea: SafeAreaInsets = getSafeAreaInsets();

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
    .padding({
      top: `${this.safeArea.top}px`,
      right: `${this.safeArea.right}px`,
      bottom: `${this.safeArea.bottom}px`,
      left: `${this.safeArea.left}px`,
    })
    .height('100%')
    .width('100%')
  }
}
