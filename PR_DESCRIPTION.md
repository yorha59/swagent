# swagent 1.32：重 IO 后监听恢复与日志故障加固

13.7 GB dump 后真机出现日志缺失、TCP 可建立但应用不响应，以及后续拒绝连接。本次加入监听健康探测、命令行读取截止时间及日志文件恢复；版本统一升至 1.32。不部署、不 commit。

## 改动

- 独立优先级 48 的监听 watchdog（普通工作线程为 44），每 30 秒检查 getsockname、SO_ACCEPTCONN、SO_ERROR。accept 循环超过 60 秒没有心跳也判为停滞；在原生 listener 所有权锁内 shutdown/close，通知 accept 循环用全新 socket/bind/listen 重建。记录关闭、socket、bind、listen 及失败日志。bind 失败无限指数退避，sys 初始 250 ms、NRO 初始 5 s，上限 30 s。
- 客户端从连接建立或上一条完整命令起，60 秒没有完整命令就断开。只发半条命令也不会延长截止时间。上传仍保留原有 30 秒收包超时。连接退出用 Drop 归还 session socket，容量满的未接入 socket 也走 Drop。
- 日志 writer 保留文件句柄，连续 10 次失败后关闭并重开一次，重试当前行；恢复成功后可开始下一轮故障恢复。持续失败不阻塞日志生产者，Rust ring 继续记录。写入将超过 1 MiB 时截断重写。
- `io::remove` 返回 Result，通过与 rename 相同的 rc() 传播错误，同时传播路径过长错误。已追查调用者：dump/安装/上传/更新/睡眠保护清理均显式忽略其 best-effort 清理结果；`files::Sd::remove` 和用户 rm 命令逻辑不改。

## Socket 泄漏排查结论

本次静态排查未发现 dump/install 大流量路径存在确定的 socket 泄漏，不能据此将真机 bsdsocket 故障归因于 fd 泄漏，也不能排除服务内部资源劣化。

- dump 本体从 NCM/已安装内容读取并写 SD，不创建出站 socket；进度、状态经已有 session 回传。文件和 stage 服务已有 Drop。
- install/update 的 HTTP 每次请求在 `Socket::connect` 成功后立即由 Rust Socket 持有；header、body、range 重试、大小校验、取消、超时及文件写失败的 `?` 返回均经过 Socket::Drop。native 连接建立阶段尚未转交 Rust，fcntl/connect/poll/超时失败统一走 failed → sa_close；DNS 链表统一 freeaddrinfo。
- 原生 track_socket 注册所有连接中/已接入 socket，32 槽满时立即关闭新 fd；sa_close 清空槽位后关闭。新增生产函数 host 回归验证 1024 次登记/释放及满槽拒绝后的 fd 确实关闭。原有连接器回归检查 3 次超时、成功及拒绝连接的 close 计数。
- 已接入 socket 归 session 所有，reply/file_reply 的 ManuallyDrop 包装只是借用，SessionIo::Drop 释放 sending 状态，由 client Connection::Drop 或显式 quit/disconnect 关闭所有者。容量拒绝路径使用 Socket::Drop。
- listener 单独由 native listener_fd 登记与加锁关闭；watchdog 只关闭仍登记为 listener 的 fd。恢复通知与 accept 错误交错时，serve 会先释放可能已重建的 listener 再创建，避免丢失句柄。网络 epoch 保护已有出站 Socket，不关闭旧 BSD 实例中已回收的描述符。

## 验证

`make check` 通过：66 个 Rust 测试、NSP/NRO 产物校验、1 个 NRO packer、3 个 NRO entry、2 个 lifecycle、1 个 connect 和 3 个 recovery 测试。完整原文见 `build/check-1.32.log`。新增 host 用例覆盖空连接/半条命令被踢、健康探测失败重建、mock 时间下 accept 心跳停滞、原生探测所有权、日志失败重开/持续失败/1 MiB 轮转、socket registry 释放和 io::remove 错误传播。socketpair 全链用生产 admission/framing/disconnect 路径，将命令截止参数缩短到 40 ms。

## 实证范围与限制

未部署、未跑真机 13.7 GB dump。元数据探测加心跳不能证明内核 backlog 的端到端可用性；若 bsdsocket IPC、探测或 SD 文件调用本身永久不返回，用户态关闭/重开机制不能保证解除该服务级死锁。watchdog 依赖 shutdown/close 让停滞的 accept 返回，再由原 accept 循环重建；未通过无限创建线程绕过永久挂起的 IPC。落盘日志仍受非阻塞队列容量和 SD 可写性约束；失败时 ring 保留最近记录。

## 构建产物

- `dist/atmosphere/contents/430000000000000B/exefs.nsp`
- `build/swagent.nsp`
- `dist/switch/swagent.nro`
- `build/swagent.elf` / `build/swagent-nro.elf`
