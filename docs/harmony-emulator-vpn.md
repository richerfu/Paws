# DevEco 模拟器 VPN 验收

本页用于本机 DevEco HarmonyOS 模拟器。先用 `hdc list targets` 确认目标；本次 Pura 90
实例的 HDC key 是 `127.0.0.1:5555`；OpenHarmony QEMU 使用单独的 HDC 目标，见文末。

2026-09-18 又在独立的 `Paws VPN Fresh` 模拟器 (`127.0.0.1:15564`) 和 OpenHarmony
QEMU (`127.0.0.1:5578`) 复验了当前 Paws 代码。QEMU 的首次授权弹窗与已有授权连接
都能走通。全新 DevEco 镜像没有授权弹窗：debug 授权 API 返回 `true` 后仍不创建授权记录，
首次连接会在启动期限后显示“VPN 异常”，且没有 `:vpn`/TUN。按下节为该**实例**预置授权
后，连接、断开和重连均通过；不能把空授权记录的 DevEco 镜像描述为开箱即用。

## 启动链路

debug HAP 在 `paws.vpn` 插件安装时调用 `updateVpnAuthorizedState`，处理缺少系统 VPN
授权弹窗的模拟器镜像。一次用户连接意图只向系统发送一次无 FD 的 VPN Want，其中只有
attempt ID 和随机的一次性 token。Extension 启动后通过本机 AF_UNIX socket 和
`SCM_RIGHTS` 接收原有 ashmem/通知 FD，再验证 owner journal 与 UI 状态、绑定 owner、
创建真实 `vpn-tun`，最后启动 Paws 数据面。release HAP 不执行 debug 授权。

系统授权记录保存在模拟器 `userdata`，但本次 Pura 90 实例在没有 Paws 授权记录时，
它返回 `true` 后系统仍报 `connectAbility failed 2097152`。这说明当前镜像不能依赖该
调用完成首次建档。不能仅凭系统 start Promise 返回来宣称 VPN 已连接。首次使用此镜像
需要先按下节预置该实例的授权记录。

## 首次授权预置

本次目标实例为 `/Volumes/PSSD/huawei/deployed/Pura 90`，原始 backing file 是 SDK 的
`userdata.img`。只修改实例自己的 `userdata.img.qcow2`，并保留修改前的完整备份。

现已提供可重复执行的 [预置脚本](../scripts/provision-harmony-emulator-vpn.py)。先安装并
启动一次 debug HAP，使 SettingsData 数据库生成；然后正常关闭模拟器，确认 `hdc`
目标消失，再运行（新实例的 user ID 为 `100`）：

```sh
python3 scripts/provision-harmony-emulator-vpn.py \
  --instance-dir '/Volumes/PSSD/huawei/deployed/Paws VPN Fresh' --user-id 100
```

脚本检查实例已停止，备份并校验原 overlay，只在临时 raw 上恢复 ext4 journal、修改
SettingsData 主库和 slave 库，恢复 uid/gid、mode、ACL、SELinux 与其他原有扩展属性，
完成 SQLite/fsck/qcow2 校验后替换实例 overlay。需要本机的 `qemu-img` 和
Homebrew `e2fsprogs`。它不会修改 SDK 基础镜像；成功后会打印备份路径。下面是脚本
执行的关键步骤，便于审计或手动恢复。

1. 安装 debug HAP 并启动一次模拟器，让 SettingsData 数据库生成；随后用 DevEco
   `Emulator -stop 'Pura 90' -instancePath /Volumes/PSSD/huawei/deployed` 正常关闭。
   `hdc list targets` 应不再出现该实例，`lsof userdata.img.qcow2` 应为空。
2. 备份实例 overlay，执行 `qemu-img check`，再用 `qemu-img convert -f qcow2 -O raw`
   合并 overlay 与原 backing file，得到临时 `userdata.raw`。不能在模拟器持有镜像时操作。
3. 本次 raw ext4 的 journal 尚待恢复且 bitmap 校验不通过；在**临时 raw 文件**上先执行
   `e2fsck -fy userdata.raw`，确认随后 `debugfs` 能正常打开。原实例 overlay 和备份
   不做 fsck。
4. 用 `debugfs` 从
   `/app/el1/0/database/com.ohos.settingsdata/entry/rdb/` 导出
   `settingsdata.db`、`settingsdata_slave.db` 和各自 WAL。记录原文件的 mode、uid、gid、
   `system.posix_acl_access`、`user.security`（若存在）和 `security.selinux`。先让 SQLite
   checkpoint WAL，再向两份数据库的 `SETTINGSDATA` 表执行下列 SQL，并分别运行
   `PRAGMA integrity_check`：

   ```sql
   INSERT INTO SETTINGSDATA(KEYWORD, VALUE) VALUES ('com.richerfu.paws', '1')
   ON CONFLICT(KEYWORD) DO UPDATE SET VALUE=excluded.VALUE;
   INSERT INTO SETTINGSDATA(KEYWORD, VALUE) VALUES ('com.richerfu.paws_100', '1')
   ON CONFLICT(KEYWORD) DO UPDATE SET VALUE=excluded.VALUE;
   ```

5. 在临时 raw 中删除旧的两份数据库及其 `-wal`、`-shm`、`-dwr`，写回更新后的数据库，
   逐项恢复原属性。当前主库的 `user.security=s1`，slave 没有该属性；两者均为
   uid/gid `20003`、mode `0660`、SELinux `u:object_r:appdat:s0`。其他镜像必须以自身
   原值为准。重新导出并核对两条授权记录，再执行 `e2fsck -fy` 和 `e2fsck -fn`，直到
   文件系统无错误。
6. 使用 `qemu-img convert -f raw -O qcow2 -B <原 userdata.img> -F raw` 生成新的实例
   overlay；`qemu-img check` 通过后替换实例的 `userdata.img.qcow2`。冷启动模拟器，
   再执行下节 smoke。回滚时先关模拟器，再恢复修改前的完整 overlay 备份。

`_100` 中的 `100` 是本次实例的用户 ID，不能直接套用到其他实例。备份留在该实例目录：
`userdata.img.qcow2.before-paws-vpn-auth-20260918-2000`。

## 本机验收命令

从仓库根目录运行。打包命令会完整重编 native `.so`，不能只执行 Hvigor 后复用旧库。

```sh
HDC_TARGET=127.0.0.1:5555
hdc -t "$HDC_TARGET" shell 'param get const.product.model; uname -m'

NATIVE_PROFILE=debug HAP_BUILD_MODE=debug scripts/package-hap.sh

scripts/harmony-smoke.sh --no-build \
  --hap entry/build/default/outputs/default/entry-default-unsigned.hap \
  --target "$HDC_TARGET" --protocol-mode direct --auto-start-vpn \
  --mock-bind 0.0.0.0 --mock-advertise-host 10.0.2.2 \
  --require-protect-success --hilog-seconds 45
```

若目标拒绝 unsigned HAP，可使用下文的测试签名脚本，给该目标的 UDID 签名后将
`--hap` 指向签名产物。`PAWS_TEST_KEYSTORE_PASSWORD=123456` 只用于 SDK 自带的公开
测试密钥，不用于发布包。

`10.0.2.2` 是本机模拟器访问宿主机 mock 服务的网关；先用
`hdc -t "$HDC_TARGET" shell 'ping -c 1 -W 1 10.0.2.2'` 确认可达。
`--allow-vpn-unsupported` 只用于定位启动请求，不用于 VPN 验收。

验收还应核对：

```sh
hdc -t "$HDC_TARGET" shell 'ps -A | grep paws'
hdc -t "$HDC_TARGET" shell 'ifconfig vpn-tun'
hdc -t "$HDC_TARGET" shell 'hilog -x | grep -E "PawsVpn|UpdateVpnAuthorize" | tail -100'
```

日志应覆盖 debug 授权、`descriptor-free system VPN bootstrap`、`onCreate`、FD handoff、
`bound VPN owner`、`created tun fd`、`protected process network` 和 `VPN start completed`。
smoke 脚本会等 VPN 连接成功后才执行本地回显服务的代理延迟和 TCP echo。系统
`vpn-tun`、应用 `:vpn` 进程和回显流量都成立，才能认定这一实例的 VPN 流程走通。

## 2026-09-18 本机结果

上述授权预置后，debug unsigned HAP 在 DevEco Pura 90、API 24、HDC
`127.0.0.1:5555` 上通过 direct smoke。日志依次出现无 FD 启动、Extension
`onCreate`、FD handoff、owner 绑定、`created tun fd 39`、
`protected process network`、`VPN start completed`，随后才有 TCP echo 成功记录。
`vpn-tun` 地址为 `172.19.0.1/30`。从系统浏览器访问宿主机
`http://192.168.3.28:8765/` 返回 HTTP 200；这次访问前后 TUN 计数从
`RX 0/TX 11` 变为 `RX 4/TX 15`。在 Paws UI 中断开后 `:vpn` 进程结束且 TUN 地址
移除，再次点击连接产生新 attempt 并重新创建 TUN。测试结束后已从 UI 断开。

最终严格 smoke 日志：`smoke-logs/paws-smoke-20260918-201606.hilog`。这里验证的是已预置授权的
当前实例；debug API 在空授权记录上的首次建档能力并未得到证明。

## QEMU 首次授权与状态对照

本次 QEMU 为 API 26、HDC `127.0.0.1:5578`。它要求带代码签名的 HAP，DevEco
模拟器使用的 unsigned HAP 会在安装时被拒绝。先完整构建 debug HAP，再用 SDK
公开测试密钥为**当前产物**生成仅供目标 QEMU 安装的测试包：

```sh
NATIVE_PROFILE=debug HAP_BUILD_MODE=debug scripts/package-hap.sh
PAWS_TEST_KEYSTORE_PASSWORD=123456 scripts/sign-harmony-test-hap.py \
  entry/build/default/outputs/default/entry-default-unsigned.hap \
  --output smoke-logs/paws-qemu-debug-signed.hap --target 127.0.0.1:5578
scripts/harmony-smoke.sh --no-build \
  --hap smoke-logs/paws-qemu-debug-signed.hap --target 127.0.0.1:5578 \
  --protocol-mode direct --auto-start-vpn --mock-bind 0.0.0.0 \
  --mock-advertise-host 10.0.2.2 --require-protect-success --hilog-seconds 75
```

首次测试前确认 QEMU 的 SettingsData 没有 `com.richerfu.paws` 授权记录；启动后系统会
显示 VPN 授权框，需在 smoke 捕获期间点击“允许”。此前 Paws 等待系统 start Promise
结束才发送 FD，导致用户允许后 Extension 已 `onCreate` 却一直等不到 FD。现在系统
启动请求与 FD handoff 并行，交接线程持续检查精确 attempt 是否仍有效；系统明确拒绝
或已接受启动但 Extension 未连接时会终止等待。应用状态仍以 Extension 发布的终态
为准。

本次首次授权 strict smoke 通过：
`smoke-logs/paws-smoke-20260918-212648.hilog`。实际弹窗允许后出现
`onCreate`、FD handoff、owner 绑定、`vpn-tun 172.19.0.1/30`、网络保护、
`VPN start completed`，随后 TCP echo 成功；UI 显示“已连接”。界面断开后
`:vpn` 进程及 TUN 消失，并有精确 attempt 的 cleanup 确认。已有授权时再次点击连接，
新 attempt 成功且不再出现授权框，断开也完成清理。
仓库内签名脚本生成的双目标测试包也在 QEMU 已授权状态下通过严格 smoke：
`smoke-logs/paws-smoke-20260918-215033.hilog`；测试结束后 UI 显示“未连接”，
系统没有 `:vpn` 进程或 `vpn-tun`。

全新 DevEco 实例空授权记录的失败日志为
`smoke-logs/paws-smoke-20260918-213345.hilog`，UI 在启动期限后显示“VPN 异常”，
无 Extension/TUN。运行预置脚本并冷启动后，该实例的严格 smoke 日志为
`smoke-logs/paws-smoke-20260918-214223.hilog`；UI 与系统的连接、断开、重连
状态一致。
