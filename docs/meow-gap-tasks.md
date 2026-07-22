# HMeta 与 Meow 差异任务清单

更新时间：2026-07-16

对照基线：

- 本项目：`/Volumes/PSSD/code/harmony/paws`
- Meow Android：`madeye/meow`，`main` = `df7ab80ca3f5e8c57bb9992da85ed308a7b4a230`
- Meow 官网说明：Flutter UI + Kotlin `VpnService` + Rust FFI + `meow-rs` + `netstack-smoltcp`

本文只记录功能和实现差异，并拆成本地可执行任务。Meow iOS 不作为主基线，只在 Meow Android 注释中出现时作为参考。

## 当前项目概况

HMeta 当前已经具备一条可运行的最小闭环：

- HarmonyOS `VpnExtensionAbility` 创建 TUN，并把 fd 传给 Rust。
- Rust `hmeta_core` 负责配置导入、配置 reload、代理选择、延迟测试和运行时 snapshot；导入/刷新会先将 Clash YAML、base64 订阅和常见分享链接归一为 meow-rs YAML，再进入 crates.io `meow-* 0.18.0` 校验。默认协议 feature 与额外 AnyTLS 已启用，覆盖 SS、Trojan、VLESS/Vision/Encryption、VMess、Snell、Hysteria2、ECH tunnel 和 AnyTLS。
- Rust `hmeta_profile` 负责 profile/rule 本地文件存储、运行时 YAML 合成、VPN 参数推导。
- Rust `hmeta_vpn` 使用 `netstack-smoltcp` 将 TUN TCP/UDP 流量转发到 `meow_tunnel`。
- Rust `hmeta_ui` 使用 arkit 构建原生 UI，并通过 NAPI 暴露 start/stop/reload/import/select 等能力。
- `local-protocol-tests` 已提供本地 echo/mock server 和 Clash YAML profile，用于手工验证 Direct/HTTP/SOCKS5/Trojan/VLESS 等协议路径。

## 总体差异

Meow 更像一个完整客户端产品，HMeta 当前更像一个核心链路已经跑通的早期 HarmonyOS 移植版。主要差异集中在：

- Meow 有成熟的 Android service 状态机、通知、AIDL IPC、EventChannel 实时状态和流量推送；HMeta 目前是单个 `VpnExtensionAbility` + Rust snapshot 轮询。
- Meow 启动 `meow-rs` API Server，并让 Flutter UI 通过 `/proxies`、`/connections`、`/logs`、`/traffic` 等 HTTP/WS 接口工作；HMeta 已启动 meow external-controller，UI 的代理选择、测速和 provider refresh 已优先走 controller，仍保留 NAPI snapshot 作为 Harmony UI 快捷路径。
- Meow 有 per-socket `VpnService.protect(fd)` JNI hook，避免代理出站连接重新进入 VPN；HMeta 目前调用 HarmonyOS `protectProcessNet()`，尚未确认是否覆盖所有 meow-rs 出站 socket。
- Meow 有 Room 数据库、订阅备份、YAML 编辑器、流量历史、per-app 列表；HMeta 当前使用 JSON + YAML 文件存储，功能更轻。
- Meow 对配置做了 Android 侧清洗：移除 app 管理的端口、替换 DNS、注入 GeoX URL、拷贝 GeoX 资源；HMeta 当前在 Rust 中 patch runtime YAML，并已接入 GeoX 私有路径/rawfile seed 桥，但真实 GeoX 资源文件和 DNS 策略还不完整。
- Meow 有 Android emulator E2E；HMeta 已增加 HarmonyOS HAP 安装/启动、14 模式本地协议矩阵、订阅 UI、设置/关于 UI 和 VPN 生命周期自动化。当前 Harmony 模拟器不创建系统 VPN TUN，因此最新构建仍需真机完成 TUN/protect/外部应用流量验收。

## P0 任务

### HMETA-MEOW-001：确认并补齐出站 socket 保护

差异：

- Meow 在 Rust FFI 中通过 JNI 调 Android `VpnService.protect(fd)`，并在 patched `meow-rs-proxy` 的 pre-connect hook 中对每个出站 socket 做保护。
- HMeta 目前在 `HMetaVpnExtensionAbility.ets` 中调用 `vpnConnection.protectProcessNet()`，并已将成功/失败回传到 Rust snapshot/log/UI，便于真机确认进程级保护是否覆盖 `meow-rs` 创建的所有 TCP/UDP 出站 socket；是否足以替代 Android per-socket protect 仍需真机验证。

落地任务：

- 调研 HarmonyOS `VpnConnection.protectProcessNet()` 的作用范围，确认是否可替代 Android per-socket protect。（已接入可观测状态，真机覆盖范围待验证）
- 如果不够，设计 Rust 到 ArkTS/Native 的 fd protect callback，并在 meow-rs 出站 dial 前调用。
- 对 TCP、UDP、DNS、profile download、provider download、proxy delay 分别建立验证用例。（snapshot/UI 已展示出站保护状态，具体链路用例待补）

验收：

- VPN 开启后，代理出站连接不会被自身 TUN 回环捕获。
- 本地 mock profile 下 Direct/HTTP/SOCKS5/Trojan/VLESS 的 delay 或 echo 验证稳定通过。
- 真机抓日志可看到出站保护路径被触发或确认无需触发。

### HMETA-MEOW-002：统一 engine lifecycle 和 running 语义

差异：

- Meow 的 engine lifecycle 是 `start engine -> start tun2socks -> service running`，并有 Kotlin `BaseService` 管理 Connecting/Connected/Stopping/Stopped。
- HMeta 已将 snapshot 语义拆开：`engineLoaded`/兼容字段 `running` 表示 meow-rs engine 已加载，`vpnRunning` 才表示真实 TUN/VPN 连接状态；同时新增 `vpnLifecycle` 汇总 `stopped`、`engine-loaded`、`starting`、`connected`、`protect-failed` 状态，UI 连接按钮绑定 `vpnRunning`，首页连接状态绑定 `vpnLifecycle`；连接主按钮已区分 profile reload、系统 VPN 启动/停止请求和本地 stop fallback 的反馈，避免把平台回调问题混成笼统 VPN 操作失败。

落地任务：

- 将 `running` 明确定义为 core engine 是否已加载，或移除/重命名为 `engineLoaded`。（已新增 `engineLoaded`，并保留 `running` 作为兼容别名）
- 将 UI 上的连接状态只绑定到 `vpn_running` 或新增显式 `VpnLifecycle`。（已绑定 `vpnRunning`，并新增 `vpnLifecycle` 用于首页状态展示和外部 snapshot 判断）
- 增加 start/reload/stop 的状态转移测试。（已覆盖 reload 只加载 engine、start/stop VPN、platform VPN 状态；UI 主按钮反馈已覆盖启动请求、启动回调失败、停止请求和停止 fallback）

验收：

- reload 配置后 UI 不误显示 VPN 已连接。
- start/stop/reload 的 snapshot 字段语义稳定，有测试覆盖。

### HMETA-MEOW-003：完善 DNS 处理策略

差异：

- Meow 禁用 meow-rs 配置里的 DNS listener，由 tun2socks 截获 UDP/53，并通过内置 DNS/DoH、China DNS、DNS cache、DNS table 回填域名。
- HMeta 当前 patch `dns.enable = true`、`listen = 127.0.0.1:1053`，同时 `hmeta_vpn` 会在 `tun.dns-hijack` 开启时对 TUN UDP/53 调 `meow_dns::DnsServer::handle_query`，关闭时让 DNS 包继续走普通 UDP 转发路径；Settings 已可编辑并保存当前 profile 的 `dns.nameserver`、`fallback`、`nameserver-policy` 和 `tun.dns-hijack` 开关，runtime YAML 与 Harmony VPN extension fallback 默认值都会内置 China DNS、fallback 和 `geosite:cn` / `geosite:geolocation-!cn` 默认分流策略，并由 App 接管 `default-nameserver` bootstrap、清理未设置的 fallback/policy，以及订阅残留的 `enhanced-mode` / `fake-ip-*` / `fallback-filter` 等与 TUN DNS 模型冲突字段，同时强制关闭 meow-rs DNS 的系统 hosts 读取，只保留配置内 `hosts` 映射，避免订阅里的本机/不可达 bootstrap、旧分流字段或设备系统 hosts 污染运行 DNS；RuntimeSnapshot/UI 已展示 DNS 模型、TUN DNS 地址、上游、fallback、分流策略数量、已拦截查询数、DNS cache hit/miss 和最近 A/AAAA 查询，最近查询会按大小写不敏感域名聚合同一条目；TUN DNS 已有有界轻量响应缓存和 IP->域名回填表，缓存 key 会按大小写不敏感的 DNS question 归一化，命中时回写当前 query 的 question section、新 transaction id 与剩余 TTL，CNAME 链最终 A/AAAA 记录会回填到原始查询域名，并在容量满时淘汰最早过期项，但还没有 Meow 的完整 DoH cache。

落地任务：

- 明确 HarmonyOS 版本采用哪种 DNS 模型：默认 TUN 截获，并保留 `dns-hijack` 关闭时走 meow-rs DNS listener/普通 UDP 的路径。（已完成配置语义和 Settings 开关接入）
- 支持 UI/配置设置 DoH/普通 DNS 上游，并写入 `VpnOptions`。（已支持 Settings 保存 `dns.nameserver` / `fallback` / `nameserver-policy` 到当前 profile 并 reload）
- 评估是否引入 Meow 的 China DNS、DoH cache、diagnostics 设计。（已补基础 diagnostics snapshot/UI、手动 split DNS 策略、内置 China DNS 默认策略、runtime DNS 清理 fake-ip/fallback-filter 残留字段、大小写不敏感聚合的最近 DNS 查询记录和有界 TUN DNS 响应缓存；缓存按 DNS question 归一化命中，并按当前 query 回写 question/transaction id 与剩余生命周期 TTL，DNS cache hit/miss 已透传 RuntimeSnapshot、Traffic/DNS 和 Settings 摘要，完整 DoH cache 仍待设计）
- 给 A/AAAA、失败响应、域名回填 TCP/UDP metadata、UDP session 建立测试。（已有 A/AAAA/错误响应/CNAME 回填/响应缓存/域名回填/过期清理基础单测，resolver 失败和 DNS burst overflow 均会回 ServFail，TCP/UDP metadata 均已覆盖 DNS table host 回填，TCP metadata 也覆盖无 host 时保留目标 IP，UDP session 保活/idle 清理、响应读端转发与异常清理、TUN UDP 包构造/解析 roundtrip、IPv4 长度校验、分片包绕过 direct DNS 截获与截断包拒绝已覆盖，UDP proxy session 已覆盖 meow-rs DIRECT 到本地 UDP echo 服务的正向发包、回包与 session 复用）

验收：

- 开 VPN 后 DNS 查询可稳定走配置的 DNS 上游。
- 通过域名访问代理节点时，规则匹配能拿到 host，而不是只看到目标 IP。
- DNS 错误响应不导致应用长时间卡住。

## P1 任务

### HMETA-MEOW-004：补齐 meow external-controller/API 能力

差异：

- Meow 启动 `ApiServer` 在 `127.0.0.1:9090`，UI 通过 REST/WS 使用 `/proxies`、`/group/*/delay`、`/connections`、`/providers/*`、`/rules`、`/logs`、`/traffic`。
- HMeta 当前保留 NAPI `RuntimeSnapshot` 快捷路径；core reload 后已启动 `meow_api::ApiServer` 并接入当前 `Tunnel`、raw config、provider maps 和 log broadcaster，已验证 `/proxies`、`/providers/*`、`/rules`、`/connections`、`/logs`、`/traffic` 的最小链路，UI 的运行模式切换会同步 core 结果并提示失败，代理选择、单节点测速、批量节点测速、provider refresh 和 Connections 关闭操作已优先切到 controller，Proxies 页面支持按节点名、类型、延迟和当前选中状态搜索。

落地任务：

- 在 `hmeta_core` 中启动 `meow_api::ApiServer`，并接入当前 `Tunnel`、raw config、provider maps、log broadcaster。（已启动 ApiServer 并接入 Tunnel/raw config/provider maps/log broadcaster）
- 保留现有 NAPI snapshot 作为 Harmony UI 快捷路径，或逐步改成读取 controller。（UI 代理选择、单节点测速、批量节点测速、单连接关闭和全部连接关闭已优先走 controller，失败回退本地 core；Proxies 页面已支持节点搜索和筛选空状态）
- 对 `/proxies`、`/proxies/{name}/delay`、`/providers/*`、`/connections`、`/logs`、`/traffic` 建立最小验证。（已验证 `/version`、`/proxies`、`/proxies/DIRECT/delay`、`/providers/proxies`、`/providers/rules`、`/rules`、`/connections`、`/logs`、`/traffic`）

验收：

- App 内可通过 `127.0.0.1:9090` 获取 proxies/rules/connections。
- UI 代理选择、单节点 delay 测试、批量 delay 测试和 Connections 关闭操作优先走 meow API，失败可回退本地 core。
- logs/traffic 有实时流式数据，不只依赖轮询 snapshot。

### HMETA-MEOW-005：配置清洗、GeoX 资源和 provider 缓存对齐

差异：

- Meow 启动前会清洗 YAML：移除 `subscriptions`，替换/禁用 DNS，移除端口配置，强制 `mixed-port`，注入 `geox-url`，并拷贝 `geoip.metadb`、`geosite.dat`、`Country.mmdb`、`GeoLite2-ASN.mmdb`。
- HMeta 当前 patch `mixed-port`、`external-controller`、`dns`、`tun`，会移除用户配置里的 listener/controller/auth/routing 等 App 管理字段（包括端口、`listeners`、`authentication`、`skip-auth-prefixes`、`routing-mark`、`interface-name`、`allow-lan` / `lan-allowed-ips` / `lan-disallowed-ips`、`external-controller*`、`external-ui*`、`tproxy-sni`），导入/刷新/编辑保存前的 meow-rs 校验也会先按同一 App-managed 语义清洗这些字段，避免最终不会生效的原始 listener、端口或 DNS bootstrap 残留阻断导入；并 rewrite provider path 到 profile 私有缓存目录，对 provider 名称生成安全文件名，避免用户配置中的 path 或 provider 名称逃逸 App 管理目录；rule-provider `inline` 会保留 `payload`，但不会注入/保留仅用于文件缓存或远程刷新的 `path` / `interval`，也不会参与“一键刷新全部 provider”；已创建 App 私有 `geodata` 目录、在 runtime YAML 注入 meow-rs 实际读取的 `geodata.mmdb-path` / `asn-path` / `geosite-path`，并丢弃订阅自带的 `geodata.auto-update`、`auto-update-interval`、`url` 和上游兼容字段，避免订阅触发 meow-rs 后台 GeoData 下载或用非法更新间隔阻断 reload；启动期已通过 `ohos-resource-manager-binding` 从 `rawfile/geodata` seed 缺失的 `Country.mmdb`、`GeoLite2-ASN.mmdb`、`geosite.dat`，并提供 `scripts/fetch-geodata.sh` / `docs/geodata.md` 用于 release 构建前拉取真实资源；RuntimeSnapshot 与 Resources 页面已展示 GeoData 三个文件的存在状态、路径、大小、人类可读更新时间和离线资源就绪/缺失摘要；Profile snapshot/Profiles 页面已展示原始 YAML 与 runtime YAML 路径，便于排查清洗后的实际配置；provider 刷新已走 meow external-controller，Resources 页面支持按资源名、类型、URL/path、刷新错误、provider interval/filter/exclude-filter/behavior/format/health-check 元数据和规则内容搜索筛选，支持单个刷新和一键刷新全部 provider，批量刷新会同步本轮成功/失败数量，刷新请求按 proxy/rule provider 类型路由，避免同名 provider 刷错 controller 集合，并在 `ProviderSummary` 展示成功/失败状态、刷新时间、错误摘要、实际缓存路径是否存在、缓存大小、更新时间、provider 更新间隔/filter/exclude-filter/behavior/format/health-check 设置，以及刷新失败时是否保留可用旧缓存。

落地任务：

- 增加 GeoX 资源打包目录和启动时 seed 逻辑。（已接入 App 私有 `geodata` 目录、runtime YAML 读取路径、rawfile seed 桥和资源拉取脚本；release runner 需要在打包前提供真实资源）
- 在 runtime YAML 构建中注入缺省 `geox-url`。（已完成）
- 梳理 `proxy-providers` / `rule-providers` 的 path、download、refresh、失败缓存策略。（refresh 已接 external-controller 并记录日志，支持单 provider 和全部可刷新 provider 刷新，runtime provider path/cache 状态已在 snapshot/UI 可见，provider path 已限制在 profile 私有缓存目录；inline rule-provider 保留 payload 且不携带 path/interval，避免与 meow-rs inline provider 加载语义冲突，批量刷新会跳过 inline，单独刷新 inline 会返回明确 skipped 错误；Resources 页面已展示并支持搜索 provider interval/filter/exclude-filter/behavior/format/health-check 元数据；刷新失败时若缓存文件仍存在，会在 snapshot/UI 标记“旧缓存可用”并保留缓存 metadata）
- 避免用户 profile 中的 `mixed-port`、`socks-port`、`port`、listener、auth、routing/interface 和 external UI/controller 设置与 App 内端口和 Harmony 网络环境冲突。（已覆盖 runtime YAML 清洗测试）

验收：

- 包含 GEOIP/GEOSITE/rule-provider 的真实订阅可离线首次启动。
- provider refresh 成功/失败都有 UI 状态或日志。（已完成，Resources 页面展示 provider 刷新状态/时间/错误摘要和缓存状态；单个 provider 刷新失败会更新卡片并弹出错误）
- runtime YAML 文件可追踪，且不会破坏用户原始 YAML。（已在 Profile snapshot/Profiles 页面暴露原始 YAML 与 runtime YAML 路径）

### HMETA-MEOW-006：订阅/Profile 存储模型升级

差异：

- Meow 的 `ClashProfile` 包含 `url`、`yamlContent`、`yamlBackup`、`selectedProxy`、`tx/rx`、`lastUpdated`。
- Paws 当前 `ProfileDocument` 包含 id/name/source/raw_yaml_path/subscription_url/updated_at，并已补齐 YAML backup、selector 选择持久化、profile 级 tx/rx 累计；规则仍单独存储，Resources 页面已可查看、排序、启停、删除当前 profile 的自定义规则并 reload；Harmony 系统备份配置已收窄到 `files/paws` 用户数据，并排除可重建的 runtime YAML、provider cache 和 GeoData。

落地任务：

- 已增加 profile 原始 YAML backup，用于编辑后 revert。
- 已持久化 selector group 的用户选择，并在 reload/start 后恢复。
- 已增加 profile 级 `tx/rx` 累计字段。
- 已支持在 Resources 页面导入、排序、启停、删除自定义规则，变更后 reload 当前 profile，运行中 VPN 会请求重启，并在 reload 或重启回调失败时保留规则写入结果同时明确显示失败原因。
- 已将 profile index、原始 YAML、备份 YAML 和规则数据纳入系统备份恢复路径，并排除 runtime/provider/geodata 等可重建缓存。
- 已对导入、刷新、编辑、回滚、删除建立基础测试。

验收：

- 用户编辑 YAML 后可以回滚到订阅原文，Profile 页面已提供回滚入口。
- VPN 重启后仍保留上次选中的节点。
- profile 删除会清理 runtime/provider/rule 关联文件。

### HMETA-MEOW-007：per-app VPN（暂不接入）

差异：

- Meow 支持 installed apps 列表、proxy/bypass 两种 per-app 模式，并在 Android `VpnService.Builder` 中配置 allowed/disallowed applications。
- HMeta 暂不接入 Harmony 的按应用 allow/block 能力。不同系统版本对空白名单和默认值的解释可能导致全部应用进入 VPN 或全部应用绕过 VPN，因此运行时 `VpnConfig` 完全不携带 `trustedApplications` / `blockedApplications` 字段。
- Settings 不提供分应用入口，应用列表读取桥接和 `GET_BUNDLE_INFO` / `GET_BUNDLE_INFO_PRIVILEGED` 权限均已移除；历史配置中的相关 YAML 字段保留原文但不会再被解析为运行参数。

落地任务：

- 若未来重新评估，必须先在目标 HarmonyOS 版本和真机上验证字段缺失、空数组、allowlist 与 blocklist 的系统语义。
- 重新接入前需要定义升级策略，确保历史配置不会在版本切换时意外改变全局 VPN 覆盖范围。

验收：

- 默认和配置启动路径都不向系统提交按应用名单。
- 旧 YAML 中的 per-app 字段不会影响当前运行。
- 安装包不申请读取应用列表权限。

### HMETA-MEOW-008：连接、日志、流量视图接入真实数据

差异：

- Meow UI 有 Traffic、Connections、Logs 页面，数据来自 EventChannel 和 meow API。
- HMeta UI 已有页面结构和 snapshot 字段；当前 Connections 已从 `meow_tunnel::Statistics` 读取活动 TCP 连接、meow-rs `start` 开始时间、结构化 `rulePayload` 和 `chains` 代理链，可按 host/rule/rulePayload/proxy/chains/network/开始时间搜索，并可在 UI 中断开指定连接或一键断开全部连接，关闭操作会优先调用 meow external-controller `/connections` DELETE，失败再回退本地 statistics；Requests 已维护最近请求/规则命中历史，连接关闭后仍能看到 host/network/rule/proxy/tx/rx/更新时间，支持搜索和活跃/已结束筛选，活跃请求可直接跳到带 host 筛选的 Connections 页面，且页面可清空历史；Traffic 同时暴露 TUN 与 meow-rs 总流量，展示当前活动 profile 的跨会话累计上传/下载，并由 core snapshot 暴露有界本机会话速度历史，UI 展示采样数、峰值上传/下载和最近速度；平台 VPN 停止回调会先收口当前 TUN/meow-rs 流量样本，避免异常停止或异步 stop 中间路径漏记最后一段 profile 累计流量；Logs 已接入进程内 tracing 环形缓冲并可在页面清空，支持等级筛选和关键词搜索，日志时间已转成人类可读 UTC 时间，meow external-controller 的 `/logs` WebSocket broadcast 已验证。

落地任务：

- 从 meow tunnel/API 接入活动连接列表。（已接入 tunnel statistics，并展示 meow-rs start 开始时间；API 复用待 HMETA-MEOW-004）
- 在 Connections 页面支持断开活动连接。（已通过 UI 调用 core close_connection / close_all_connections）
- Requests 页面保留最近请求和规则命中历史。（已基于 tunnel statistics 生成 `requestHistory`，活跃连接消失后标记为已结束，页面支持搜索、状态筛选、tx/rx 和更新时间展示，NAPI 支持清空）
- 接入 tracing/log broadcast，替代仅内存手写 log。（已接入 tracing 环形缓冲，并验证 external-controller `/logs` WebSocket；页面和 NAPI 均支持清空状态日志与 runtime 环形日志）
- 流量统计区分 TUN 层 tx/rx、meow-rs connection 层 tx/rx、profile 累计 tx/rx。（snapshot/UI 已展示 TUN、meow-rs、当前 profile 累计流量，core snapshot 已下发有界本机会话速度历史，UI 展示历史摘要；profile 累计已在 VPN stop、平台 stopped 回调、active profile 切换前和 active profile 删除前收口当前样本，并对 meow-rs 统计做 baseline，避免停止后 snapshot 切换主统计源或切换/删除 profile 时重复入账/串账；长时间重启/跨进程历史语义仍需继续校准）

验收：

- Connections 页面能看到 host/network/rule/rulePayload/proxy/chains/tx/rx/开始时间，并可断开活动连接。
- Logs 页面能实时显示 core/proxy/DNS 错误。
- Traffic 页面重启后能显示会话和历史统计。

## P2 任务

### HMETA-MEOW-009：YAML 编辑器能力

差异：

- Meow 集成 Sora Editor 和 TextMate YAML 语法高亮，支持保存、回滚、测试。
- HMeta 当前支持从 URL/文件/内容导入 profile，Profiles 页面已提供 YAML 查看/编辑/保存/回滚能力，编辑器会显示行数、字符数、代理/规则/Provider 数量、未保存修改和 YAML 可解析状态，回滚备份会显示成功/失败反馈，并可在保存前手动测试当前 YAML 是否能被 meow-rs 加载；尚未集成 Sora Editor/TextMate 高亮。

落地任务：

- 在 Rust UI 中增加 profile YAML 查看/编辑页面。（已提供 Profiles 卡片入口和弹窗编辑器）
- 保存前调用 `meow_config::load_config_from_str` 校验。（已复用 core 校验，并在编辑器提供独立“测试”动作）
- 保存后支持 reload 当前 profile，失败则回滚。（已保存后 reload；语法错误不会写入，active reload 失败会回滚旧 YAML）

验收：

- 用户可以查看、编辑、测试、保存、回滚 YAML。
- 编辑时可以快速确认内容规模、常见 Clash 配置段数量、是否有未保存修改以及 YAML 是否可解析。
- 语法错误不会破坏当前可用配置。

### HMETA-MEOW-010：订阅刷新和批量刷新体验

差异：

- Meow 有 add/update/delete/select/refreshAll 订阅操作。
- HMeta 有 import URL、refresh 单 profile、delete、activate；Profiles 顶部已增加批量刷新和到期刷新入口，启动时会静默刷新到期订阅，Profile 卡片已提供删除入口，导入/启用/删除配置会区分 reload、数据写入/删除和运行中 VPN 停止/重启请求失败，本地配置不再显示可点击的订阅刷新动作，单订阅刷新会给出成功/失败 toast，批量刷新会统计本轮成功/失败数量，core 会逐个刷新订阅并记录单项失败，避免一个坏订阅影响其他订阅；导入/刷新已在 meow-rs 校验前支持 Clash YAML、base64 订阅和常见分享链接归一化，并保留 VLESS/Trojan/VMess 常见 WS/gRPC/h2/httpupgrade/TLS/Reality 传输参数，HTTP/SOCKS5 分享链接也已归一化到 meow-rs 当前可加载字段；profile 元数据和 Profiles 页面已展示最近刷新时间、成功/失败/待刷新状态、失败原因，以及 HTTP `subscription-userinfo`、配置头部连续注释块里的用量注释、HTTP 头中的 `profile-title` / `content-disposition` 或配置头部连续注释块中的 `profile-title`、`profile-update-interval`、`profile-web-page-url`、`support-url` 订阅元数据（HTTP 头优先，正文注释补缺字段），订阅到期/最近刷新/下次刷新时间已转成人类可读 UTC 时间，订阅首页/支持链接可通过系统 viewData 打开；Profiles 页面已支持按名称、来源、订阅地址、订阅标题/主页/支持链接、刷新状态和失败原因筛选。

落地任务：

- UI 增加批量刷新。（已完成）
- UI 增加删除订阅入口，删除当前 active profile 时自动切换到剩余配置或清空 engine 状态，运行中 VPN 会请求重启/停止。（已完成）
- 增加刷新状态、失败原因、最后更新时间。（已完成，`ProfileSummary` 暴露 `lastRefreshAt` / `lastRefreshError`，Profiles 页面展示成功/失败和错误摘要）
- 增加订阅用量和到期信息展示。（已完成，URL 导入/刷新解析 HTTP `subscription-userinfo` 和配置头部连续注释块里的 `# upload=...; download=...;` / `# subscription-userinfo: ...`，并在 Profiles 页面展示用量/总量/到期时间）
- 增加订阅标题、更新间隔、主页和支持链接展示。（已完成，URL 导入/刷新解析 HTTP 头与配置头部连续注释块中的 `profile-title`、`content-disposition`、`profile-update-interval`、`profile-web-page-url`、`support-url`，支持单行 `key=value` 和多行 `key: value`，HTTP 头优先且正文注释可补缺字段；Profiles 页面展示可用元数据，主页/支持链接可点击打开）
- 增加订阅更新间隔的到期判断。（已完成，`ProfileSummary` 暴露 `nextRefreshAt` / `refreshDue`，core/NAPI 提供 `refreshDueProfiles`，Profiles 顶部提供到期刷新入口，App 启动时会静默刷新到期订阅）
- 多订阅列表增加搜索筛选。（已完成，Profiles 页面支持按名称、来源、订阅地址、元数据、状态和错误摘要筛选）
- 刷新 active profile 后自动 reload，并保持 selector 选择。（已完成，复用单 profile refresh/reload 路径）
- 订阅内容解析对齐 Meow/FlClash 常见输入：Clash YAML、base64 多行分享链接、单条分享链接。（已完成基础归一化，VLESS/Trojan/SS/SSR/VMess/Hysteria/Hysteria2/TUIC/HTTP/SOCKS5 分享链接协议头和 query 参数名均已大小写不敏感，VMess JSON 字段名也会按大小写不敏感读取；多行分享链接订阅会跳过注释、未知行和坏节点，只要仍有有效节点就继续导入，单条坏链接仍会返回解析错误；VLESS/Trojan/SS/Hysteria/Hysteria2/TUIC/HTTP/SOCKS5 在缺少 fragment 时会兼容 `remarks` / `remark` / `name` / `ps` / `alias` / `node-name` 等 query 节点名别名，fragment 仍优先；VLESS/Trojan/SS/SSR/VMess/Hysteria/Hysteria2/TUIC 分享链接已支持常见 WS/gRPC/h2/httpupgrade/TLS/Reality 参数，并兼容常见别名如 `serverName` / `servername`、`wsPath` / `wsHost`、WS early-data `ed` / `eh`、`client-fingerprint` / `clientFingerprint`、`grpc-service-name` / `grpc-mode`、`allow-insecure=allow` / `allow_insecure=true`；VLESS `security=TLS` / `security=Reality`、`tls=true` / `enable-tls=1` 大小写兼容并会写出 `tls: true`，`encryption=none` 会归一化保留；VLESS/Trojan/SS/SSR 会保留 `udp=false` / `udp=0` / `udp=off` 等显式 UDP 关闭语义；Reality 会保留 `public-key`、`short-id` 和 `spider-x`；VMess 归一化会保留 skip-cert-verify、`allow_insecure` / `insecure`、gRPC service/mode、UDP/TFO 开关，并兼容 `name` / `remarks`、`server` / `address`、`uuid`、`network`、数字形态的 `port` / `aid`，且 `security=tls` 会作为 TLS 开关而不是 cipher 写出；SS 已支持 SIP002 userinfo 形态、旧式整段 `method:password@host:port` base64 形态，并保留 simple-obfs/v2ray-plugin 内联参数、显式 `plugin-opts` / `pluginOpts` 和 TFO 开关，且插件名和参数名大小写不敏感；SSR URI 已保留 cipher、password、protocol、protocol-param、obfs、obfs-param 和 group，并兼容 `Remark`/`ProtocolParam`/`ObfsParam`/`GroupName` 等常见 query 别名；Hysteria URI 已保留 auth-str、protocol、SNI/peer、ALPN、skip-cert-verify、obfs、up/down、端口跳跃、窗口和 MTU/FastOpen 参数；Hysteria2 URI 已保留 SNI/peer、ALPN、skip-cert-verify、混淆、up/down、端口跳跃、窗口、MTU 和 FastOpen 参数，并兼容 query 中的 `password` / `auth` / `auth-str` 密码别名；TUIC URI 已保留 SNI、ALPN、skip-cert-verify、拥塞、UDP relay、disable-sni、reduce-rtt、timeout、heartbeat、UDP packet size 和 FastOpen 参数；HTTP/SOCKS5 分享链接已支持认证、TLS 和 skip-cert-verify，HTTP 会保留 `headers` / `header` 查询里的 CONNECT header map，SOCKS5 还会保留 UDP/TFO 开关，其中 VLESS 的 WS/h2/httpupgrade/Vision/encryption/udp-off 与 TFO、Trojan gRPC 与 TFO、SS simple-obfs/v2ray-plugin 与 TFO、HTTP/SOCKS5 已经 meow-rs reload 验证；当前 `meow-* 0.18.0` 已完成 VMess、Hysteria2、Snell、AnyTLS 等配置 reload 覆盖，SSR、Hysteria v1 与 TUIC 不在当前 meow-config feature 集内，归一化后的此类输入会返回明确的内核不支持错误而不会伪装导入成功）

验收：

- 多订阅场景下可一键刷新全部。
- 单个失败不影响其他订阅。
- 单个订阅刷新失败后，Profiles 页面可看到失败状态和失败原因。（已完成）

### HMETA-MEOW-011：本地化与产品页面补齐

差异：

- Meow 有中英文 Flutter 文案和商店/隐私页面。
- HMeta 当前 UI 文案主要是中文；关于页已展示 App/Core 版本、meow-rs 与 arkit commit，并说明订阅、本地配置、流量/日志/DNS 统计默认本机处理；UI 已增加 zh-CN/en 字符串表入口，EntryAbility 会将 Harmony 系统语言同步给 native UI，导航标题、Dashboard 首页、Proxies 页筛选/空态/节点状态、Profiles 页筛选/空态/订阅卡片状态与导入/YAML 编辑弹层及其操作反馈、VPN/模式/代理测速/连接断开/请求历史/日志清理/规则/provider/配置刷新/设置保存与校验、内部 VPN 回调错误和 DNS policy 解析错误等运行时反馈、Requests/Connections 运行态列表、Traffic/DNS 诊断页、Resources 页 provider/规则/GeoData 状态、Logs 页筛选/空态、Tools 页固定入口、Settings 页 VPN/DNS/分应用设置和 About 页稳定文案已接入。

落地任务：

- 梳理 UI 字符串资源，支持至少 zh-CN/en。（已建立 `l10n` 字符串表和 `configureUiLocale` / `HMETA_UI_LOCALE` 入口，ArkTS 启动和系统配置更新时会同步系统语言；先覆盖导航、Dashboard 首页、Proxies 页筛选/空态/节点状态、Profiles 页筛选/空态/订阅卡片状态与导入/YAML 编辑弹层及其操作反馈、VPN/模式/代理测速/连接断开/请求历史/日志清理/规则/provider/配置刷新/设置保存与校验、内部 VPN 回调错误和 DNS policy 解析错误等运行时反馈、Requests/Connections 运行态列表、Traffic/DNS 诊断页、Resources 页 provider/规则/GeoData 状态、Logs 页筛选/空态、Tools 页固定入口/运行摘要、Settings 页 VPN/DNS/分应用设置和 About 页）
- 增加隐私说明：订阅、本地配置、流量数据均本地处理。（已完成，About 页展示）
- 增加关于页、版本号、meow-rs commit 展示。（已完成，同时展示 arkit commit 和 Rust 版本）

验收：

- UI 文案逐步从业务逻辑迁移到字符串表；导航、Dashboard 首页、Proxies 页筛选/空态/节点状态、Profiles 页筛选/空态/订阅卡片状态与导入/YAML 编辑弹层及其操作反馈、VPN/模式/代理测速/连接断开/请求历史/日志清理/规则/provider/配置刷新/设置保存与校验、内部 VPN 回调错误和 DNS policy 解析错误等运行时反馈、Requests/Connections 运行态列表、Traffic/DNS 诊断页、Resources 页 provider/规则/GeoData 状态、Logs 页筛选/空态、Tools 页固定入口、Settings 页 VPN/DNS/分应用设置和 About 页已有 zh-CN/en 覆盖。
- 关于页可看到 app 版本、core 版本、依赖 commit。（已完成）

### HMETA-MEOW-012：发布与 CI

差异：

- Meow 有 GitHub Actions lint/tests/release、Fastlane、Android E2E。
- HMeta 当前已提供本地 `scripts/verify.sh` 串联 Rust fmt/test、local-protocol profile 生成回归与 `ohrs build --arch aarch`；`scripts/package-hap.sh` 会在 `ohrs build --arch aarch` 后复制最新 `libhmeta_ui.so` 并通过 DevEco/hvigor 无签名打出 `entry-default-unsigned.hap`，默认传 `--no-daemon` 避免 Hvigor daemon 锁影响本地验证；GitHub Actions 已拆分 Rust hosted job 与 HarmonyOS self-hosted job，Rust job 覆盖本地协议 profile 生成。

落地任务：

- 增加 Rust test/check/fmt CI。（已完成，见 `.github/workflows/ci.yml`）
- 增加 HarmonyOS hvigor build CI，至少产出 debug HAP。（已完成，self-hosted runner 上运行 `ohrs build --arch aarch` 并上传 HAP artifact）
- 整理签名失败场景和本地签名文档。（已完成，见 `docs/ci-and-release.md`）

验收：

- PR/本地脚本可一键跑 Rust + HarmonyOS 构建。（本地见 `scripts/verify.sh`；无签名 HAP 打包见 `scripts/package-hap.sh`；PR 见 CI workflow）
- CI artifact 可下载 HAP。（self-hosted HarmonyOS job 上传 `*.hap`）

## 测试任务

### HMETA-MEOW-013：基于 local-protocol-tests 扩展 App 手动验收矩阵

差异：

- Meow E2E 使用 `ssserver` + Android emulator + 预置数据库 + UI 自动操作。
- HMeta 已有 `local-protocol-tests` 的 echo/mock server 和 profile 模板；`docs/app-protocol-acceptance.md` 已形成 App 侧手动验收矩阵，覆盖 direct/http/http-auth/http-bad-auth/http-down/socks5/socks5-auth/socks5-bad-auth/ss/ss-bad-password/trojan/trojan-bad-password/vless/vless-bad-uuid 的导入、启用、VPN 启动、测速和 echo payload 预期。

落地任务：

- 为 direct/http/http-auth/socks5/socks5-auth/ss/trojan/vless/vless-bad-uuid 写手动验收步骤。（已完成，见 `docs/app-protocol-acceptance.md`；并扩展 http-bad-auth/socks5-bad-auth/ss-bad-password/trojan-bad-password 认证负路径）
- 每个模式记录 import、reload_config、start VPN、test_proxy_delay、echo payload 的预期结果。（已完成）
- 增加失败用例：错误 UUID、错误 HTTP/SOCKS auth、错误 SS/Trojan password、mock server 断开。（已完成）

验收：

- `docs/` 中有一份可直接照跑的 App 协议验收矩阵。
- 每次 VPN 改动都能用该矩阵回归。

### HMETA-MEOW-014：自动化 E2E 探索

差异：

- Meow 有 Android emulator 脚本，可以自动安装、导入订阅、启动 VPN、验证外网。
- HMeta 已有 HarmonyOS 真机 smoke 脚本，可构建/安装/启动 EntryAbility、通过 debug Want 参数导入 profile、请求 VPN 启动并导出 hilog；脚本也可通过 `--protocol-mode` 自动启动 `local-protocol-tests`、生成 profile、导入并启动 VPN，且会校验 TUN 创建、出站保护日志、native proxy delay 和 TCP echo payload 结果；`scripts/harmony-protocol-matrix.sh` 可一次循环 direct/http/http-auth/http-bad-auth/http-down/socks5/socks5-auth/socks5-bad-auth/ss/ss-bad-password/trojan/trojan-bad-password/vless/vless-bad-uuid，并默认要求 `protectProcessNet()` 成功；`--require-protect-success` 可用于 HMETA-MEOW-001 真机验收，要求 `protectProcessNet()` 明确成功；安装前会校验 HAP 内 `libhmeta_ui.so` 包含 `Index.d.ts` 声明的 NAPI 导出，并默认 force-stop App，避免签名包带旧 native 库或连续协议矩阵复用旧进程；`--device-probe-command` 可在 VPN 启动后运行设备侧外部进程探针并按输出/退出码验收，第三方 App/helper 的标准化命令矩阵仍待补。

落地任务：

- 调研 DevEco/hdc 是否能自动安装、启动 Ability、传入 Want 参数、读取 hilog。（已完成基础 `hdc install` / `aa start` / `hilog` smoke）
- 用本地 mock profile 作为自动化输入，先实现 smoke：安装 -> 导入 profile -> 启动 VPN -> 检查日志。（已完成安装 -> 启动 -> profile 导入/reload -> 请求 VPN 启动 -> TUN/protect hilog 标记检查 -> native delay/echo 正负向结果检查；`--protocol-mode` 可自动拉起本地 mock profile；`scripts/harmony-protocol-matrix.sh` 可循环全部本地协议模式；`--require-protect-success` 可将出站保护从“有明确结果”收紧为“必须成功”）
- 后续再接标准化第三方 App/helper 经 TUN 发包验证。（已有 `--device-probe-command` 外部进程探针入口，具体 helper/命令矩阵待定）

验收：

- 一条脚本能完成 HAP 安装和 VPN 启动 smoke。（已完成，见 `scripts/harmony-smoke.sh --profile ... --auto-start-vpn` 或 `scripts/harmony-smoke.sh --protocol-mode http --auto-start-vpn`）
- 失败时自动导出 hilog。（已完成，输出到 `smoke-logs/`）

## 实施顺序建议

1. 先做 `HMETA-MEOW-001` 和 `HMETA-MEOW-002`，保证 VPN 不回环、状态语义准确。
2. 再做 `HMETA-MEOW-003`、`HMETA-MEOW-004`、`HMETA-MEOW-005`，让 core 行为接近完整 meow-rs 客户端。
3. 接着做 `HMETA-MEOW-006`、`HMETA-MEOW-007`、`HMETA-MEOW-008`，补用户可感知能力。
4. 最后做 YAML 编辑、本地化、CI/E2E，把产品体验和工程稳定性补齐。

## 当前已接近 Meow 的部分

- VPN 私有地址段已经沿用 Meow：`172.19.0.1/30`、`172.19.0.2`、`fdfe:dcba:9876::1/126`。
- TUN 到 `meow_tunnel` 的核心 TCP 路径已经存在，并使用 `netstack-smoltcp`。
- UDP session 保活/idle 清理、响应读端异常清理、DNS 响应回写、TUN stats 已有基础实现和基础单测。
- profile 导入、刷新、激活、provider path rewrite、rules 管理已有基础实现；订阅解析已覆盖 Clash YAML、base64 文本订阅、VLESS/Trojan/SS/VMess 分享链接归一化，其中 VLESS WS/h2/httpupgrade/Vision、Trojan gRPC、HTTP/SOCKS5 有 core reload 测试覆盖。
- 本地协议 mock 测试目录已经覆盖 Meow/meow-rs 当前主要 embedded protocol test 风格。

## 当前风险

- 如果 `protectProcessNet()` 不能覆盖 meow-rs 出站 socket，真机 VPN 会出现自吞流量或连接超时；当前已能在 UI/snapshot/logs 看到保护调用是否成功，Harmony smoke 也能用 `--require-protect-success` 强制保护成功，但仍需按链路确认覆盖范围。
- engine loaded 与 VPN connected 语义已拆分为 `engineLoaded`/`running` 与 `vpnRunning`，后续仍需在真机 smoke 中持续回归连接按钮和平台 VPN 状态同步。
- DNS 模型尚未定型，真实订阅中的 GEOIP/GEOSITE/DoH/provider 组合可能不稳定。
- Connections/Logs/Providers 页面仍以 snapshot 渲染为主，后续可继续收敛到 external-controller 数据模型。
- HarmonyOS E2E 已有 `hdc shell` 外部进程探针入口，但仍缺标准化第三方 App/helper 经 TUN 发包矩阵；后续每次改 VPN 仍需要结合 smoke 和手工矩阵回归。
