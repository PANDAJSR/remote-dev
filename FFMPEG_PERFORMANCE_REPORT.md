# WebRTC 远程桌面性能优化报告

## 一、当前状况

### 1.1 已实现的功能

- ✅ **关键帧检测修复**：NAL Type 检测从 IDR(5) 扩展到 IDR+SPS+PPS
- ✅ **编码器重试机制**：空数据时强制关键帧重试
- ✅ **单例捕获器**：多会话共享同一屏幕捕获器（广播模式）
- ✅ **并行颜色转换**：使用 Rayon 实现多线程 BGRA→I420
- ✅ **性能计时日志**：详细的各环节耗时监控
- ✅ **FFmpeg 集成**：自动检测 NVENC/QuickSync/AMF 硬件编码器

### 1.2 性能瓶颈分析

| 组件 | 当前耗时 | 瓶颈原因 |
|------|----------|----------|
| **BGRA→I420 转换** | ~35ms | 1080p 分辨率计算量大 |
| **H.264 编码** | ~100-300ms | openh264 软件编码，单线程 |
| **FFmpeg 进程启动** | ~200-300ms | **每帧都启动新进程** |
| **实际帧率** | **3-5 FPS** | 远低于目标 30 FPS |

### 1.3 根本问题

**当前 FFmpeg 实现缺陷**：

```rust
// 当前实现（每帧启动进程 - 错误！）
pub fn encode_frame(&mut self, bgra: &[u8]) -> Result<Vec<u8>> {
    // 1. 启动 FFmpeg 进程 (~200ms)
    let mut child = Command::new("ffmpeg").spawn()?;
    
    // 2. 写入数据
    child.stdin.write_all(&i420)?;
    
    // 3. 等待进程结束 (~100ms)
    let output = child.wait_with_output()?;
    
    Ok(output.stdout) // 总耗时: ~300ms/帧 = 3 FPS
}
```

**正确做法**：

```rust
// 应该使用持久化进程
let mut encoder_process: Child; // 启动一次，重复使用

pub fn encode_frame(&mut self, bgra: &[u8]) -> Result<Vec<u8>> {
    // 直接写入 stdin，立即返回
    encoder_process.stdin.write_all(&i420)?;
    // 从 stdout 读取编码数据
    encoder_process.stdout.read(&mut buffer)?;
    // 总耗时: ~10ms/帧 = 100 FPS
}
```

---

## 二、持久化 FFmpeg 重构方案

### 2.1 架构设计

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│   ScreenCapture │     │  FFmpegEncoder   │     │   WebRTC Track  │
│   (BGRA 帧)     │────→│  (持久化进程)     │────→│  (H.264 数据)   │
└─────────────────┘     └──────────────────┘     └─────────────────┘
                                │
                    ┌───────────┴───────────┐
                    ▼                       ▼
            ┌─────────────┐         ┌─────────────┐
            │  stdin pipe │         │ stdout pipe │
            │  (I420 输入)│         │(H.264 输出) │
            └─────────────┘         └─────────────┘
                    │                       │
                    ▼                       ▼
            ┌─────────────────────────────────────┐
            │       FFmpeg Child Process          │
            │  h264_nvenc / h264_qsv / libx264   │
            │  启动一次，持续运行整个会话          │
            └─────────────────────────────────────┘
```

### 2.2 核心实现

```rust
// backend/src/rdp/ffmpeg_encoder.rs

use std::process::{Command, Stdio, Child};
use std::io::{Write, Read};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;
use std::time::Duration;

pub struct FFmpegEncoder {
    width: usize,
    height: usize,
    frame_count: u64,
    
    // 持久化进程
    ffmpeg_process: Child,
    
    // 通信通道
    input_sender: Sender<Vec<u8>>,
    output_receiver: Receiver<Vec<u8>>,
}

impl FFmpegEncoder {
    pub fn new(width: usize, height: usize, bitrate: u32) -> anyhow::Result<Self> {
        let encoder_type = Self::detect_best_encoder();
        
        // 启动持久化 FFmpeg 进程
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .arg("-hide_banner")
            .arg("-loglevel").arg("error")
            .arg("-f").arg("rawvideo")
            .arg("-pix_fmt").arg("yuv420p")
            .arg("-s").arg(format!("{}x{}", width, height))
            .arg("-r").arg("30")
            .arg("-thread_queue_size").arg("1024")
            .arg("-i").arg("-");
        
        // 编码器配置
        match encoder_type.as_str() {
            "h264_nvenc" => {
                cmd.arg("-c:v").arg("h264_nvenc")
                    .arg("-preset").arg("fast")
                    .arg("-rc").arg("vbr")
                    .arg("-cq").arg("25");
            }
            _ => {
                cmd.arg("-c:v").arg("libx264")
                    .arg("-preset").arg("ultrafast")
                    .arg("-tune").arg("zerolatency");
            }
        }
        
        cmd.arg("-b:v").arg(format!("{}", bitrate))
            .arg("-g").arg("30")
            .arg("-f").arg("h264")
            .arg("-");
        
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        
        let mut child = cmd.spawn()?;
        
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        
        // 创建通信通道
        let (input_tx, input_rx) = channel::<Vec<u8>>();
        let (output_tx, output_rx) = channel::<Vec<u8>>();
        
        // 启动输入线程
        thread::spawn(move || {
            let mut stdin = stdin;
            while let Ok(data) = input_rx.recv() {
                if stdin.write_all(&data).is_err() {
                    break;
                }
            }
        });
        
        // 启动输出线程
        thread::spawn(move || {
            let mut stdout = stdout;
            let mut buffer = vec![0u8; 65536];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(n) if n > 0 => {
                        let _ = output_tx.send(buffer[..n].to_vec());
                    }
                    _ => break,
                }
            }
        });
        
        Ok(Self {
            width,
            height,
            frame_count: 0,
            ffmpeg_process: child,
            input_sender: input_tx,
            output_receiver: output_rx,
        })
    }
    
    pub fn encode_frame(&mut self, bgra: &[u8]) -> anyhow::Result<Vec<u8>> {
        // 1. BGRA → I420（并行）
        let i420 = Self::bgra_to_i420(bgra, self.width, self.height);
        
        // 2. 发送给 FFmpeg（非阻塞）
        self.input_sender.send(i420)?;
        
        // 3. 接收编码数据
        let encoded = self.output_receiver.recv_timeout(
            Duration::from_millis(100)
        )?;
        
        self.frame_count += 1;
        Ok(encoded)
    }
    
    fn bgra_to_i420(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
        use rayon::prelude::*;
        
        let y_size = width * height;
        let uv_size = y_size / 4;
        let mut i420 = vec![0u8; y_size + 2 * uv_size];
        
        let (y_plane, rest) = i420.split_at_mut(y_size);
        let (u_plane, v_plane) = rest.split_at_mut(uv_size);
        
        // Y 平面并行
        y_plane.par_chunks_mut(width)
            .enumerate()
            .for_each(|(y, row)| {
                let bgra_row = &bgra[y * width * 4..(y + 1) * width * 4];
                for (x, y_val) in row.iter_mut().enumerate() {
                    let idx = x * 4;
                    let b = bgra_row[idx] as i32;
                    let g = bgra_row[idx + 1] as i32;
                    let r = bgra_row[idx + 2] as i32;
                    *y_val = ((76 * r + 150 * g + 29 * b) >> 8).clamp(0, 255) as u8;
                }
            });
        
        // UV 平面并行
        let half_w = width / 2;
        let half_h = height / 2;
        
        let uv_data: Vec<(usize, u8, u8)> = (0..half_h)
            .into_par_iter()
            .flat_map(|y| {
                let y2 = y * 2;
                (0..half_w).into_par_iter().map(move |x| {
                    let x2 = x * 2;
                    let idx00 = (y2 * width + x2) * 4;
                    let idx01 = idx00 + 4;
                    let idx10 = ((y2 + 1) * width + x2) * 4;
                    let idx11 = idx10 + 4;
                    
                    let b = (bgra[idx00] as i32 + bgra[idx01] as i32 + 
                             bgra[idx10] as i32 + bgra[idx11] as i32) >> 2;
                    let g = (bgra[idx00 + 1] as i32 + bgra[idx01 + 1] as i32 + 
                             bgra[idx10 + 1] as i32 + bgra[idx11 + 1] as i32) >> 2;
                    let r = (bgra[idx00 + 2] as i32 + bgra[idx01 + 2] as i32 + 
                             bgra[idx10 + 2] as i32 + bgra[idx11 + 2] as i32) >> 2;
                    
                    let u = ((-44 * r - 87 * g + 131 * b) >> 8) + 128;
                    let v = ((131 * r - 110 * g - 21 * b) >> 8) + 128;
                    
                    (y * half_w + x, u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
                }).collect::<Vec<_>>().into_par_iter()
            })
            .collect();
        
        for (idx, u_val, v_val) in uv_data {
            u_plane[idx] = u_val;
            v_plane[idx] = v_val;
        }
        
        i420
    }
    
    fn detect_best_encoder() -> String {
        use std::process::Command;
        
        let encoders = [
            ("h264_nvenc", "NVIDIA NVENC"),
            ("h264_qsv", "Intel QuickSync"),
            ("h264_amf", "AMD VCE"),
        ];
        
        if let Ok(output) = Command::new("ffmpeg")
            .args(&["-hide_banner", "-encoders"])
            .output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for (name, desc) in &encoders {
                if stdout.contains(name) {
                    println!("[FFMPEG] Found hardware encoder: {} ({})", name, desc);
                    return name.to_string();
                }
            }
        }
        
        println!("[FFMPEG] Using software encoder: libx264");
        "libx264".to_string()
    }
}

impl Drop for FFmpegEncoder {
    fn drop(&mut self) {
        // 关闭时终止 FFmpeg 进程
        let _ = self.ffmpeg_process.kill();
    }
}
```

### 2.3 关键改进点

| 改进项 | 当前方案 | 持久化方案 | 预期收益 |
|--------|----------|------------|----------|
| **进程启动** | 每帧启动 (~300ms) | 启动一次 | **+25 FPS** |
| **管道通信** | 无（进程退出即结束） | 持续 stdin/stdout | **低延迟** |
| **编码器初始化** | 每帧重新初始化 | 一次初始化，重复使用 | **GPU 保持加载** |
| **内存分配** | 频繁分配释放 | 预分配缓冲区 | **减少 GC 压力** |

### 2.4 预期性能

基于 NVIDIA NVENC 硬件编码能力：

| 分辨率 | 编码耗时 | 理论帧率 |
|--------|----------|----------|
| 1920x1080 | ~8-12ms | 80-120 FPS |
| 1280x720 | ~4-6ms | 160-250 FPS |

**实际可达帧率**：
- **1080p @ 30-60 FPS**（受限于 PCIe 带宽和 GPU 负载）
- **720p @ 60+ FPS**（轻松达到）

### 2.5 重构工作量

| 任务 | 复杂度 | 预估时间 |
|------|--------|----------|
| 持久化 FFmpeg 进程封装 | 中 | 2 小时 |
| 线程安全的输入/输出管道 | 中 | 1.5 小时 |
| 错误处理和进程重启机制 | 中 | 1 小时 |
| 性能测试和调优 | 低 | 1 小时 |
| **总计** | | **~6 小时** |

### 2.6 风险与注意事项

1. **进程崩溃处理**：FFmpeg 可能因非法输入崩溃，需要自动重启机制
2. **内存泄漏**：长期运行的进程需要监控内存使用
3. **帧同步**：需要确保输入/输出帧一一对应，避免错乱
4. **平台兼容性**：Windows 管道实现与 Linux/macOS 略有不同

---

## 三、建议的实施路径

### Phase 1: 快速验证（1-2 天）

1. 实现基本的持久化 FFmpeg 编码器
2. 测试 NVENC 在 1080p 下的实际性能
3. 验证帧同步是否正确

### Phase 2: 稳定化（2-3 天）

1. 添加错误处理和自动重启
2. 实现内存监控和泄漏防护
3. 优化管道缓冲区大小

### Phase 3: 调优（1-2 天）

1. 根据 GPU 型号调整编码参数
2. 测试长时间运行稳定性
3. 对比 1080p vs 720p 的实际用户体验

---

## 四、替代方案对比

| 方案 | 开发时间 | 性能 | 复杂度 | 推荐度 |
|------|----------|------|--------|--------|
| **持久化 FFmpeg** | 6 小时 | 30-60 FPS | 中 | ⭐⭐⭐⭐⭐ |
| **Media Foundation** | 16 小时 | 30-60 FPS | 高 | ⭐⭐⭐⭐ |
| **NVIDIA SDK 直接调用** | 24+ 小时 | 60+ FPS | 很高 | ⭐⭐⭐ |
| **降低分辨率到 720p** | 0.5 小时 | 15-20 FPS | 低 | ⭐⭐⭐⭐ |

---

## 五、结论

**当前 openh264 方案**能稳定工作但帧率不足（3-5 FPS）。

**推荐立即实施持久化 FFmpeg 方案**，原因：

1. **开发成本相对较低**（6 小时）
2. **性能提升显著**（可达 30-60 FPS）
3. **自动利用硬件加速**（NVENC/QuickSync/AMF）
4. **无需修改现有架构**，仅替换编码器实现

**下一步行动**：是否需要我开始实施持久化 FFmpeg 重构？

---

## 附录：当前架构代码位置

- **屏幕捕获**：`backend/src/rdp/capture.rs`
- **编码器接口**：`backend/src/rdp/capture.rs` (Vp8Encoder)
- **FFmpeg 尝试**：`backend/src/rdp/ffmpeg_encoder.rs`
- **WebRTC 集成**：`backend/src/rdp/webrtc.rs`
- **性能日志**：已集成到 `encode_frame` 方法

## 附录：依赖项

当前 `Cargo.toml` 已包含：
- `openh264 = "0.9"` - 软件编码备用
- `rayon = "1.10"` - 并行计算
- `ffmpeg` (系统依赖) - 硬件编码

---

*报告生成时间：2025年2月*
