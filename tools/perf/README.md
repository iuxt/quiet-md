# 资源压测（macOS）

先构建 `cargo build --release --locked --features devtools`。Python 需要 Pillow（供样例生成器生成图片）。所有正文都是生成的测试数据；每个场景顺序运行，各用一个临时文档库，不使用系统剪贴板或抢焦点。

```sh
python3 tools/perf/stress_suite.py /tmp/quiet-stress-new
python3 tools/perf/stress_suite.py /tmp/quiet-stress-selected --cases million,photos,library-10000
```

输出目录应是新的。场景覆盖小文档持续编辑和空闲、百万 / 五百万字符混合文档、撤销重做、长 Markdown、长段落、万行代码、千行表格、千篇 / 万篇文档库、大量搜索命中、40 次窗口开关、24 张 4032×3024 图片、两轮切换 160 篇文档。某场景超时或失败后继续其余场景，最终返回非零退出码；超时会尝试抓栈，只结束直接启动的测试子进程。

单独采样一份 harness 脚本：

```sh
python3 tools/perf/resource_probe.py script.txt synthetic-library /tmp/quiet-probe-new
```

源文档库只读复制到输出的 `lib/`；`QUIET_MD_HOME`、`TMPDIR` 和日志目录指向输出目录，库的本机状态和解析缓存也隔离在其中。用 `phase 名字` 划分阶段（名字在一份脚本中应唯一），用 `idle 毫秒` 等待原生事件循环，`wait` 则会每 16ms 检查并绘制失效帧。采样器不强制绘图，也不调用系统剪贴板。光标默认正常闪烁，`--no-blink` 只用于诊断对照。

`resources.json` / `samples.jsonl` 保存每阶段汇总 / 约 100ms 一次的原始采样，`log` 保存帧耗时，`file-writes.jsonl` 记录被观察文件的替换及内容是否相同。默认观察 `quiet.json` 和文件名排序后的前四份正文；`--watch notes/<UUID>.xml` 可指定正文，`--watch-cache` 也观察本机解析缓存。轮询可能漏掉两次采样间的替换，观察次数是下界。每个阶段还比较文档库和本机缓存的文件 inode、mtime 和大小；跨阶段完成的异步写入归于实际观察到的阶段。

指标来自 `proc_pid_rusage(RUSAGE_INFO_V4)`：

- CPU 用 user + system 时间差除以墙钟时间，100% 是一个逻辑核心。时间先用 `mach_timebase_info` 从 Mach ticks 换算，不能在 Apple Silicon 上直接当纳秒。
- 内存以 `phys_footprint` 为主，另记 RSS、采样峰值和内核记录的进程存活期峰值。GPU、压缩内存及分配器保留会使它不同于 RSS；短时间稳定不能证明长期无泄漏。
- `logical_writes` 是进程逻辑写入，`diskio_byteswritten` 是内核记账的磁盘写入，不能当成 SSD 的物理 NAND 写入量；包含程序自身日志，截图也会增加它，所以资源阶段宜避免 `shot`。文件系统缓存未清空，启动测量不是冷盘测试。
- `bench` 的 step 和 frame 分别计命令处理、同步绘帧；分位数不能直接相加，也不是按键到屏幕的端到端延迟。活动操作受脚本节奏和硬件影响。

`mem_probe.py` 仍适合用 `footprint` 分解截图时刻的内存类别；README 的帧基准见 [docs/PERFORMANCE.md](../../docs/PERFORMANCE.md)。
