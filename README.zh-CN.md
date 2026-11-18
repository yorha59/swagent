# swagent

[English](README.md) | [简体中文](README.zh-CN.md)

无界面的 Atmosphere **sysmodule**：把 Switch 变成一台可脚本化的 TCP 命令代理——装 NSP、热更新自身、读写 SD、截 JPEG、查状态。

- **Title ID：** `430000000000000B`
- **端口：** `6000`（命令，CRLF）· `6001`（心跳 `pong`）
- **实现：** Rust `no_std` staticlib + 薄 C ABI shim，链官方 libnx/devkitA64
- **当前版本串：** `swagent-1.35-rs`

> **声明** — 仅供自有主机、合法内容。仓库不含 keys / 固件 / 任天堂专有代码。与 Nintendo 无关。风险自负。

## 定位

常见方案要么是完整 FTP，要么是按键代理，要么是机内 GUI 安装器。swagent 反着来：给笔记本 / CI agent 留一条**常驻命令管道**。安装不占收包线程；`ping` / `version` / `update` 保持快路径；listener 由单一 owner 负责重建，避免乱关 FD。

## 安装

### 环境

- 主机已跑 Atmosphère
- 编译机：`$DEVKITPRO`（默认 `$HOME/devkitpro`）、libnx、rustup stable + `aarch64-unknown-none`

### Release（推荐）

1. 从 [最新 Release](https://github.com/yorha59/swagent/releases/latest) 下载 `swagent.nsp`
2. 放到 SD：

```text
atmosphere/contents/430000000000000B/exefs.nsp
atmosphere/contents/430000000000000B/flags/boot2.flag   # 空文件即可
```

3. 重启进 Atmosphère 一次。之后可用在线 `update` 热替换，不必再重启。

### 源码编译

```sh
rustup target add aarch64-unknown-none
make -j2
make check
cp -R dist/atmosphere /path/to/sd_root/
```

可选 hbmenu 版：`make nro` → `dist/switch/swagent.nro` 拷到 `/switch/`。

## 上手

```sh
printf 'ping\r\nversion\r\n' | nc SWITCH_IP 6000
printf 'x' | nc SWITCH_IP 6001   # 任意字节 → pong
printf 'install http://PC:8000/game.nsp sd md5=HEX\r\n' | nc SWITCH_IP 6000
printf 'update http://PC:8000/swagent.nsp\r\n' | nc SWITCH_IP 6000
printf 'shot\r\n' | nc SWITCH_IP 6000
```

仅支持明文 HTTP（无 HTTPS / 无跳转 / 无压缩体）。

## 命令摘要

| 命令 | 作用 |
| --- | --- |
| `ping` / `version` / `status` / `log` | 存活与诊断 |
| `install` / `update` | HTTP 装包 / 热更自身 |
| `shot` / `hash` / `cat`·`put`·`ls`·`mkdir`·`rm`·`tail` | 截屏与 SD 文件 |
| `ps` / `kill` / `launch` / `quit` | 进程（`launch` 需开 am.debug） |
| `installed` / `dump` / `set` / `sleep` / `wifi` / `reboot` | 已装列表、导出、设置、整机重启 |

细节与演进记录见 [CHANGELOG.md](CHANGELOG.md)。

## 安全

仓库**不**附带 `prod.keys`。不要提交内网脚本、本机绝对路径或凭证。

## License

[MIT](LICENSE) © yorha59
