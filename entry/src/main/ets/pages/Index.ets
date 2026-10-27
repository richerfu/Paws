import { NshSurface } from '@nsh/ohos';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { PawsCoreGateway, pawsHost } from '../components/PawsCoreGateway';

@Entry
@Component
struct Index {
  private surface: NshSurface = new NshSurface(this.getUIContext());
  @State loadError: string = '';

  aboutToAppear(): void {
    const host = pawsHost();
    this.surface.registerComponent('paws.core', (handle: number) =>
      new PawsCoreGateway(this.getUIContext(), handle, this.surface, host));
    this.surface.setErrorHandler((message: string): void => {
      this.loadError = message;
      hilog.error(0x0000, 'PawsNsh', 'surface failed %{public}s', message);
    });
    this.surface.loadResource('paws.bundle.js').then((): void => {
      hilog.info(0x0000, 'PawsNsh', 'Paws bundle accepted by nsh-ohos');
    }).catch((error: Error): void => { this.loadError = error.message; });
  }

  aboutToDisappear(): void { this.surface.dispose(); }

  onBackPress(): boolean { return this.surface.onBackPress(); }

  build() {
    Stack() {
      ContentSlot(this.surface.content)
      if (this.loadError.length > 0) {
        Text(this.loadError).padding(20).fontColor('#b42318').backgroundColor('#fff4ed')
          .id('paws-load-error')
      }
    }
    .height('100%')
    .width('100%')
    // The NSH SafeAreaView consumes the measured insets. ArkUI must let its
    // background cover the system bars instead of reserving a second region.
    .expandSafeArea([SafeAreaType.SYSTEM], [SafeAreaEdge.TOP, SafeAreaEdge.BOTTOM])
    .backgroundColor(Color.Transparent)
    .onAreaChange((_previous: Area, current: Area): void => {
      hilog.info(0x0000, 'PawsNsh', 'surface bounds vp x=%{public}s y=%{public}s w=%{public}s h=%{public}s',
        String(current.globalPosition.x), String(current.globalPosition.y), String(current.width), String(current.height));
    })
  }
}
