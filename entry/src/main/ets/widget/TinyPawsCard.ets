let storage: LocalStorage = new LocalStorage();

@Entry(storage)
@Component
struct TinyPawsCard {
  @LocalStorageProp('status') status: string = '打开应用查看 VPN 状态';
  @LocalStorageProp('updated') updated: string = '';

  build() {
    Column({ space: 10 }) {
      Text('TinyPaws').fontSize(22).fontWeight(FontWeight.Bold).fontColor('#E5E7EB')
      Text(this.status).fontSize(14).fontColor('#D1D5DB').maxLines(2)
      Text(this.updated).fontSize(11).fontColor('#9CA3AF')
      Text('打开连接与诊断').fontSize(13).fontColor('#93C5FD')
    }
    .width('100%').height('100%').padding(16).alignItems(HorizontalAlign.Start)
    .justifyContent(FlexAlign.Center).backgroundColor('#111827')
    .onClick(() => {
      postCardAction(this, { action: 'router', abilityName: 'EntryAbility', params: { tinyPawsFromCard: true } });
    })
  }
}
