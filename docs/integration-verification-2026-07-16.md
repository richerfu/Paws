# HMeta 全量迁移集成验证（2026-07-16）

## 基线与结论

本轮迁移以以下本地基线为准：

- Paws：`/Volumes/PSSD/code/harmony/paws`
- Meow 参考源码：`/Volumes/PSSD/code/harmony/meow`
- Meow Android 参考包：`/Users/ranger/Downloads/meow-v1.0.1-universal.apk`
- `meow-*` crates：`0.17.0`
- arkit：`fe8f35cb4b67f126981d8ffc0c8368ec76a8bec1`，启用 `router` 与 `shadcn`

当前 Rust 核心、配置导入、订阅交互、Harmony NAPI、协议拨号和 UI 自动化已完成验证。当前 Harmony 模拟器不会实际创建 `VpnExtensionAbility` TUN，因此系统 VPN 建链、`protectProcessNet()` 和其他应用流量穿过 TUN 必须在真机上完成最后验收；模拟器不会被记录为 VPN 通过。

## 已验证范围

| 范围 | 结果 | 验证方式 |
| --- | --- | --- |
| OHOS aarch 编译与 HAP 打包 | 通过 | `scripts/package-hap.sh` |
| `meow-* 0.17.0` 配置能力 | 通过 | SS、Trojan、VLESS、AnyTLS、VMess、Snell、Hysteria2、HTTP、SOCKS5 加载测试 |
| Rust workspace | 通过 | `cargo test --workspace` |
| 本地协议 profile 生成 | 通过 | `scripts/verify-local-protocols.sh` |
| Harmony 原生协议链路 | 14/14 通过 | Direct、HTTP、SOCKS5、SS、Trojan、VLESS 的成功与错误凭据/下线分支 |
| 订阅导入后 UI | 通过 | 列表、选中态、操作菜单、编辑订阅自动化与截图 |
| 设置/关于 UI | 通过 | 行对齐、二级页导航、长文本、隐私列表、仓库入口自动化与截图 |
| VPN 启动状态机 | 通过 | `starting` 持久化、跨进程状态、15 秒超时、失败态可见 |
| 模拟器系统 VPN/TUN | 不支持 | 只有启动请求，无 `HMetaVpn`、TUN fd 或 protect 回调 |
| 当前构建真机系统 VPN/TUN | 待验收 | 按本文“真机最终验收”执行 |

Harmony 协议矩阵使用的完整命令：

```sh
scripts/harmony-protocol-matrix.sh \
  --allow-vpn-unsupported \
  --no-require-protect-success \
  --mock-bind 0.0.0.0 \
  --mock-advertise-host 192.168.3.131 \
  --hilog-seconds 20
```

通过的模式：

```text
direct
http http-auth http-bad-auth http-down
socks5 socks5-auth socks5-bad-auth
ss ss-bad-password
trojan trojan-bad-password
vless vless-bad-uuid
```

正向用例必须同时出现 delay 和逐字节 TCP echo 成功；负向用例必须出现“failed as expected”，不能吞掉错误或让 App 卡在连接中。

## 订阅导入后的 Meow 对齐项

`scripts/harmony-subscription-ui-smoke.sh` 会清理应用数据、通过 URL 导入订阅并验证：

- 首页显示导入后的当前配置。
- 订阅列表显示选中状态、名称、单行截断 URL 和更新时间。
- 点击未选中的订阅可切换当前配置。
- 更多操作保留 Meow 的核心操作集：选择、编辑订阅、编辑 YAML、导出配置、刷新订阅、恢复备份、删除配置。
- 编辑订阅使用名称/URL 表单和单一主操作，不使用大块“配置”按钮。
- 删除操作使用危险色，其余操作保持 shadcn 中性色。
- 卡片、弹窗、菜单和按钮不使用阴影。

证据文件位于 `smoke-logs/`：

- `hmeta-subscription-list.jpeg`
- `hmeta-subscription-actions.jpeg`
- `hmeta-subscription-edit.jpeg`
- 对应的 `.json` UI tree

## 设置、路由与长文本验收

`scripts/harmony-settings-ui-smoke.sh` 验证：

- “版本 / 引擎 / 分应用 VPN”共享同一图标列和标题列，不继承原生 Button 默认内边距。
- 每个路由使用独立 Scroll key，切换页面不会继承其他页面的滚动位置。
- 网络设置等二级页面显示返回按钮，并隐藏首页底部主导航。
- arkit 完整 commit 使用中段省略，前后识别信息保留且不会越过卡片边界。
- 隐私条目使用固定图标槽，换行文本保持悬挂对齐。
- meow-rs 与 arkit 仓库入口等宽，图标槽和 label 基线一致。

证据文件位于 `smoke-logs/`：

- `hmeta-settings-alignment.jpeg`
- `hmeta-network-child.jpeg`
- `hmeta-about-optimized.jpeg`
- 对应的 `.json` UI tree

## 模拟器 VPN 限制与降级行为

模拟器中 `startVpnExtensionAbility()` 可能长期不返回，也不会启动独立的 VPN extension 进程。本实现会：

1. 先持久化 `starting` 状态，供 Entry 与 VPN 两个进程共享。
2. 最多等待 15 秒。
3. 超时后写入 `failed` 状态和具体错误。
4. 首页保持“未连接”，显示“VPN 启动失败”，Switch 回到关闭状态。
5. 调试自动化的 delay/echo 不再被系统 VPN Promise 阻塞。

对应证据：

- `smoke-logs/hmeta-vpn-timeout-home.jpeg`
- `smoke-logs/hmeta-vpn-timeout-home.json`
- `smoke-logs/hmeta-smoke-20260717-000056.hilog`

日志中应有 `request VPN start received`、delay/echo 成功和 `VPN extension startup timed out`，不应有 `created tun fd`。这证明的是模拟器降级正确，不代表系统 VPN 已通过。

## 真机最终验收

### 1. 签名并安装当前 HAP

使用 DevEco Studio 或设备对应的签名配置生成 signed HAP。不要把旧 signed HAP 与最新 Rust `.so` 混用；smoke 会检查 HAP 内 NAPI 导出是否与 `Index.d.ts` 一致。

```sh
HDC_TARGET=<device-key> \
HAP_PATH=<absolute-path-to-current-signed.hap> \
scripts/harmony-smoke.sh --hilog-seconds 20
```

### 2. 跑真机协议矩阵

真机和 Mac 必须在互通网络中，使用 Mac 的局域网 IP。不要使用模拟器专用的 `10.0.2.2`，因为宿主机上的 mock proxy 还需要回连 echo server。

```sh
HDC_TARGET=<device-key> \
HAP_PATH=<absolute-path-to-current-signed.hap> \
scripts/harmony-protocol-matrix.sh \
  --mock-bind 0.0.0.0 \
  --mock-advertise-host <Mac-LAN-IP> \
  --require-protect-success \
  --hilog-seconds 20
```

每个正向用例必须具备：

- `created tun fd <n>`
- `protected process network`
- native VPN started/running 状态
- delay 与 TCP echo 成功

每个负向用例必须明确失败，且 VPN 仍可关闭并重新启动。

### 3. 验证其他进程流量穿过 VPN

HMeta 进程内 delay/echo 只能验证 meow outbound，不能代替系统 TUN 验收。至少增加一个 `hdc shell` 或测试 App 发起的外部请求：

```sh
HDC_TARGET=<device-key> \
HAP_PATH=<absolute-path-to-current-signed.hap> \
scripts/harmony-smoke.sh \
  --protocol-mode direct \
  --mock-bind 0.0.0.0 \
  --mock-advertise-host <Mac-LAN-IP> \
  --auto-start-vpn \
  --require-protect-success \
  --device-probe-command '<device-side HTTP/TCP probe>' \
  --device-probe-match '<expected payload>' \
  --hilog-seconds 20
```

同时验证：开启 VPN 后请求命中 TUN/规则/代理统计；关闭 VPN 后系统网络恢复；重复启动/停止不残留 `starting` 或错误的 `connected` 状态。

### 4. 跑 UI 回归

安装 signed HAP 后执行：

```sh
HDC_TARGET=<device-key> INSTALL_HAP=0 scripts/harmony-settings-ui-smoke.sh
HDC_TARGET=<device-key> scripts/harmony-subscription-ui-smoke.sh
```

订阅脚本默认使用 `10.0.2.2:8766`，真机运行时应通过 `PROFILE_URL` 指定真机可访问的 Mac LAN URL：

```sh
HDC_TARGET=<device-key> \
PROFILE_URL=http://<Mac-LAN-IP>:8766/direct.yaml \
scripts/harmony-subscription-ui-smoke.sh
```

真机通过以上四步后，才可将当前构建标记为完整 VPN E2E 通过。
