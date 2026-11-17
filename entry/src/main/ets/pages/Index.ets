import { NshSurface } from "@nsh/ohos";
import { hilog } from "@kit.PerformanceAnalysisKit";
import BuildProfile from "BuildProfile";
import { describeError } from "../common/Errors";
import { PawsCoreGateway, pawsHost } from "../components/PawsCoreGateway";

@Entry
@Component
struct Index {
  private surface: NshSurface = new NshSurface(this.getUIContext());
  private contentMounted: boolean = false;
  @State loadError: string = "";
  @State loadFailed: boolean = false;

  aboutToAppear(): void {
    const host = pawsHost();
    this.surface.registerComponent(
      "paws.core",
      (handle: number) => {
        const gateway = new PawsCoreGateway(this.getUIContext(), handle, this.surface, host);
        this.contentMounted = true;
        return gateway;
      },
    );
    this.surface.setErrorHandler((message: string): void => {
      this.reportError(message, !this.contentMounted || !this.surface.isRunning);
    });
    void this.loadBundle();
  }

  private reportError(message: string, loadFailed: boolean): void {
    hilog.error(0x0000, "PawsNsh", "surface failed (startup=%{public}s) %{public}s", String(loadFailed), message);
    if (this.surface.isDisposed) return;
    // Runtime errors must not leave a developer overlay on a usable release UI.
    if (BuildProfile.DEBUG) this.loadError = message;
    if (loadFailed) this.loadFailed = true;
  }

  private async loadBundle(): Promise<void> {
    if (this.surface.isDisposed) return;
    this.loadError = "";
    this.loadFailed = false;
    this.contentMounted = false;
    try {
      await this.surface.loadResource("paws.bundle.js");
      hilog.info(0x0000, "PawsNsh", "Paws bundle accepted by nsh-ohos");
    } catch (error) {
      this.reportError(describeError(error), true);
    }
  }

  aboutToDisappear(): void {
    this.surface.dispose();
  }

  onBackPress(): boolean {
    return this.surface.onBackPress();
  }

  build() {
    Stack() {
      ContentSlot(this.surface.content)
      if (this.loadFailed) {
        Column({ space: 16 }) {
          Text($r("app.string.surface_load_failed"))
            .fontColor($r("sys.color.ohos_id_color_text_primary"));
          Button($r("app.string.surface_retry"))
            .onClick((): void => { void this.loadBundle(); });
        }
        .padding(24)
        .width("100%")
        .height("100%")
        .justifyContent(FlexAlign.Center)
        .backgroundColor($r("sys.color.ohos_id_color_background"));
      }
      if (BuildProfile.DEBUG && this.loadError.length > 0) {
        Text(this.loadError)
          .padding(20)
          .fontColor("#b42318")
          .backgroundColor("#fff4ed")
          .id("paws-load-error");
      }
    }
    .height("100%")
    .width("100%")
    // The NSH SafeAreaView consumes the measured insets. ArkUI must let its
    // background cover the system bars instead of reserving a second region.
    .expandSafeArea([SafeAreaType.SYSTEM], [SafeAreaEdge.TOP, SafeAreaEdge.BOTTOM])
    .backgroundColor(Color.Transparent)
    .onAreaChange((_previous: Area, current: Area): void => {
      hilog.info(
        0x0000,
        "PawsNsh",
        "surface bounds vp x=%{public}s y=%{public}s w=%{public}s h=%{public}s",
        String(current.globalPosition.x),
        String(current.globalPosition.y),
        String(current.width),
        String(current.height),
      );
    });
  }
}
