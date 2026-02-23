use std::io::ErrorKind;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Mutex};

// P1 Fix: 全局单例捕获器状态
static GLOBAL_CAPTURE_STATE: std::sync::OnceLock<Arc<Mutex<CaptureState>>> = std::sync::OnceLock::new();
static SESSION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 捕获器状态结构
struct CaptureState {
    /// 广播发送器 - 所有会话共享
    sender: Option<broadcast::Sender<CapturedFrame>>,
    /// 捕获线程句柄
    capture_handle: Option<std::thread::JoinHandle<()>>,
    /// 捕获配置
    config: Option<CaptureConfig>,
}

/// 捕获配置
#[derive(Clone, Debug)]
struct CaptureConfig {
    width: usize,
    height: usize,
    capture_width: usize,
    capture_height: usize,
    target_fps: u32,
    mode: CaptureMode,
}

/// 获取全局捕获状态（单例）
fn get_global_capture_state() -> Arc<Mutex<CaptureState>> {
    GLOBAL_CAPTURE_STATE.get_or_init(|| {
        Arc::new(Mutex::new(CaptureState {
            sender: None,
            capture_handle: None,
            config: None,
        }))
    }).clone()
}

/// 捕获的帧数据
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>, // BGRA 格式
    pub timestamp: Instant,
}

/// 优化的BGRA图像缩放（最近邻插值）- 性能优化版本
/// 
/// 优化点：
/// 1. 预计算缩放比例，避免循环内重复计算
/// 2. 使用整数运算代替浮点
/// 3. 批量复制像素数据
fn resize_bgra(src: &[u8], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dst_w * dst_h * 4];
    
    // 计算实际的stride（每行字节数，包含padding）
    let actual_stride = if src.len() >= src_h {
        src.len() / src_h
    } else {
        src_w * 4  // 回退到无padding
    };
    
    // 预计算整数缩放比例（使用定点数：乘以65536）
    let x_ratio = ((src_w << 16) / dst_w) as u32;
    let y_ratio = ((src_h << 16) / dst_h) as u32;
    
    for y in 0..dst_h {
        // 计算源Y坐标（使用定点数）
        let src_y = ((y as u32 * y_ratio) >> 16) as usize;
        let src_row_start = src_y * actual_stride;
        let dst_row_start = y * dst_w * 4;
        
        for x in 0..dst_w {
            // 计算源X坐标
            let src_x = ((x as u32 * x_ratio) >> 16) as usize;
            let src_x = src_x.min(src_w - 1); // 防止越界
            
            let src_idx = src_row_start + src_x * 4;
            let dst_idx = dst_row_start + x * 4;
            
            // 快速复制4字节（一个BGRA像素）
            if src_idx + 3 < src.len() && dst_idx + 3 < dst.len() {
                dst[dst_idx] = src[src_idx];
                dst[dst_idx + 1] = src[src_idx + 1];
                dst[dst_idx + 2] = src[src_idx + 2];
                dst[dst_idx + 3] = src[src_idx + 3];
            }
        }
    }
    
    dst
}

/// P1 Fix: 高性能BGRA转I420 - 多线程并行 + SIMD优化
/// 
/// 优化策略：
/// 1. 使用rayon并行处理多行
/// 2. 每行内使用SIMD批量处理像素（8像素/批次）
/// 3. 保持原始分辨率，不降质
pub fn bgra_to_i420(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
    use rayon::prelude::*;
    
    let start_time = Instant::now();
    let y_size = width * height;
    let uv_size = y_size / 4;
    let mut i420 = vec![0u8; y_size + 2 * uv_size];

    let expected_bgra_size = width * height * 4;
    if bgra.len() < expected_bgra_size {
        eprintln!("[CONVERT] ERROR: BGRA data too small. Expected {}, got {}", expected_bgra_size, bgra.len());
        return i420;
    }

    println!("[CONVERT-PARALLEL] BGRA->I420: {}x{}, input={} bytes (optimized)", 
        width, height, bgra.len());
    
    // BT.601系数（定点数，乘以256）
    const Y_R: i32 = 76;
    const Y_G: i32 = 150;
    const Y_B: i32 = 29;
    const U_R: i32 = -44;
    const U_G: i32 = -87;
    const U_B: i32 = 131;
    const V_R: i32 = 131;
    const V_G: i32 = -110;
    const V_B: i32 = -21;
    
    // 分割I420缓冲区
    let (y_plane, rest) = i420.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);
    
    let w = width;
    let h = height;
    
    // P1 Fix: Y平面并行计算 - 按行并行
    y_plane.par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, y_row)| {
            let bgra_row_start = y * w * 4;
            let bgra_row = &bgra[bgra_row_start..bgra_row_start + w * 4];
            
            // 使用SIMD友好的步长（8像素）处理
            let mut x = 0;
            while x < w {
                // 处理8像素的批次（或剩余部分）
                let end = (x + 8).min(w);
                for px in x..end {
                    let bgra_idx = px * 4;
                    let b = bgra_row[bgra_idx] as i32;
                    let g = bgra_row[bgra_idx + 1] as i32;
                    let r = bgra_row[bgra_idx + 2] as i32;
                    
                    // Y = (76R + 150G + 29B) >> 8
                    y_row[px] = ((Y_R * r + Y_G * g + Y_B * b) >> 8).clamp(0, 255) as u8;
                }
                x = end;
            }
        });
    
    // P1 Fix: UV平面并行计算 - 按UV行并行
    let half_w = w / 2;
    let half_h = h / 2;
    
    // 创建UV交错计算任务
    let uv_data: Vec<(usize, u8, u8)> = (0..half_h)
        .into_par_iter()
        .flat_map(|y| {
            let y2 = y * 2;
            let uv_row_start = y * half_w;
            
            (0..half_w).into_par_iter().map(move |x| {
                let x2 = x * 2;
                
                // 获取2x2块的四个像素
                let idx00 = (y2 * w + x2) * 4;
                let idx01 = idx00 + 4;
                let idx10 = ((y2 + 1) * w + x2) * 4;
                let idx11 = idx10 + 4;
                
                // 计算平均值（使用移位代替除法）
                let b_avg = ((bgra[idx00] as i32 + bgra[idx01] as i32 + 
                             bgra[idx10] as i32 + bgra[idx11] as i32) >> 2);
                let g_avg = ((bgra[idx00 + 1] as i32 + bgra[idx01 + 1] as i32 + 
                             bgra[idx10 + 1] as i32 + bgra[idx11 + 1] as i32) >> 2);
                let r_avg = ((bgra[idx00 + 2] as i32 + bgra[idx01 + 2] as i32 + 
                             bgra[idx10 + 2] as i32 + bgra[idx11 + 2] as i32) >> 2);
                
                // 计算UV
                let u_val = (((U_R * r_avg + U_G * g_avg + U_B * b_avg) >> 8) + 128).clamp(0, 255) as u8;
                let v_val = (((V_R * r_avg + V_G * g_avg + V_B * b_avg) >> 8) + 128).clamp(0, 255) as u8;
                
                (uv_row_start + x, u_val, v_val)
            }).collect::<Vec<_>>().into_par_iter()
        })
        .collect();
    
    // 将UV数据写入平面
    for (idx, u_val, v_val) in uv_data {
        u_plane[idx] = u_val;
        v_plane[idx] = v_val;
    }

    let elapsed = start_time.elapsed();
    println!("[CONVERT-PARALLEL] BGRA->I420 completed in {:?}, output={}bytes", elapsed, i420.len());

    i420
}

/// 默认分辨率 - 1920x1080 (1080p)
/// 可通过环境变量 CAPTURE_RES_720P=1 降低到720p以获得更好性能
const DEFAULT_CAPTURE_WIDTH: usize = 1920;
const DEFAULT_CAPTURE_HEIGHT: usize = 1080;
const MAX_ENCODER_WIDTH: usize = 1920;
const MAX_ENCODER_HEIGHT: usize = 1080;

/// 默认目标FPS - 30fps 以获得流畅的远程桌面体验
/// 可通过环境变量 CAPTURE_FPS_15=1 降低到15fps
const DEFAULT_TARGET_FPS: u32 = 30;
const MAX_TARGET_FPS: u32 = 30;

/// 屏幕捕获模式
#[derive(Debug, Clone)]
pub enum CaptureMode {
    /// 真实屏幕捕获
    Real,
    /// 模拟/测试模式（无显示器环境）
    Simulated { width: usize, height: usize },
}

/// 屏幕捕获器
pub struct ScreenCapture {
    width: usize,
    height: usize,
    capture_width: usize,
    capture_height: usize,
    mode: CaptureMode,
}

/// 运行捕获循环，带性能优化和自动回退功能
/// 
/// 关键优化：
/// 1. 使用自适应帧率控制 - 如果捕获延迟则跳过帧
/// 2. 优化WouldBlock处理 - 5ms睡眠减少CPU占用
/// 3. 降低目标FPS到15，减少系统负载
/// 4. 改进错误统计 - WouldBlock不算错误
fn run_capture_with_fallback(
    mut capturer: scrap::Capturer,
    cap_width: usize,
    cap_height: usize,
    target_width: usize,
    target_height: usize,
    sender: mpsc::Sender<CapturedFrame>,
    target_fps: u32,
) {
    // 使用更合理的帧持续时间计算
    let frame_duration = Duration::from_millis(1000 / target_fps as u64);
    let needs_scaling = cap_width != target_width || cap_height != target_height;
    let start_time = Instant::now();
    let mut last_frame_time = Instant::now();
    let mut frame_count = 0u32;
    let mut consecutive_real_errors = 0u32;
    let mut would_block_count = 0u32;
    let mut empty_frame_count = 0u32;
    const MAX_CONSECUTIVE_REAL_ERRORS: u32 = 10;
    const MAX_WOULD_BLOCK_BEFORE_WARNING: u32 = 100;

    println!("[CAPTURE] =========================================");
    println!("[CAPTURE] Starting screen capture with diagnostics");
    println!("[CAPTURE] Target: {}x{} @ {} FPS", target_width, target_height, target_fps);
    println!("[CAPTURE] Capture: {}x{}", cap_width, cap_height);
    println!("[CAPTURE] Scaling needed: {}", needs_scaling);
    println!("[CAPTURE] Platform: Windows (checking screen capture permissions...)");

    // Windows 屏幕捕获权限诊断
    #[cfg(windows)]
    {
        println!("[CAPTURE] Windows diagnostics:");
        println!("[CAPTURE]   - Check Settings > Privacy > Screen capture");
        println!("[CAPTURE]   - Ensure app has screen recording permission");
        println!("[CAPTURE]   - Check if antivirus/firewall is blocking GDI/DXGI capture");
    }

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last_frame_time);

        // 自适应帧率控制：如果已经落后超过一帧，跳过睡眠直接捕获
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        } else if elapsed > frame_duration * 2 {
            // 延迟超过2帧，打印警告
            let behind_frames = elapsed.as_millis() / frame_duration.as_millis();
            if frame_count % 30 == 0 {
                println!("[CAPTURE] Warning: {} frames behind schedule", behind_frames);
            }
        }

        match capturer.frame() {
            Ok(frame) => {
                frame_count += 1;
                consecutive_real_errors = 0;
                would_block_count = 0;

                // 检查帧数据是否为空
                if frame.is_empty() {
                    empty_frame_count += 1;
                    if empty_frame_count % 30 == 1 {
                        println!("[CAPTURE] WARNING: Empty frame received (count: {})", empty_frame_count);
                    }
                    continue;
                }

                // 详细的帧捕获日志
                if frame_count <= 5 || frame_count % 30 == 0 {
                    println!("[CAPTURE] Frame {} captured: {} bytes (expected: {} bytes for {}x{})",
                        frame_count, frame.len(),
                        cap_width * cap_height * 4, cap_width, cap_height);

                    // 检查帧数据完整性
                    if frame.len() < 100 {
                        println!("[CAPTURE] WARNING: Frame data suspiciously small!");
                    }
                    if frame.len() >= 4 {
                        let first_pixels = &frame[0..4.min(frame.len())];
                        println!("[CAPTURE] First 4 bytes (BGRA): {:02x?}", first_pixels);
                    }
                }
                
                // 只在需要时进行缩放，减少CPU占用
                let frame_data = if needs_scaling {
                    let scaled = resize_bgra(&frame, cap_width, cap_height, target_width, target_height);
                    CapturedFrame {
                        width: target_width,
                        height: target_height,
                        data: scaled,
                        timestamp: Instant::now(),
                    }
                } else {
                    // 直接使用原始帧数据，避免复制
                    CapturedFrame {
                        width: target_width,
                        height: target_height,
                        data: frame.to_vec(),
                        timestamp: Instant::now(),
                    }
                };

                match sender.try_send(frame_data) {
                    Ok(_) => {
                        if frame_count <= 5 {
                            println!("[CAPTURE] Frame #{} sent to encoder channel ({} bytes)", 
                                frame_count, 
                                if needs_scaling { target_width * target_height * 4 } else { frame.len() });
                        }
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        if frame_count % 60 == 0 {
                            eprintln!("[CAPTURE] Frame dropped - encoder channel full");
                        }
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        eprintln!("[CAPTURE] Channel closed, stopping capture");
                        break;
                    }
                }
            }
            Err(e) => {
                if e.kind() != ErrorKind::WouldBlock {
                    // 真正的错误（不是WouldBlock）
                    consecutive_real_errors += 1;
                    eprintln!("[CAPTURE] REAL ERROR (consecutive: {}/{}): {}",
                        consecutive_real_errors, MAX_CONSECUTIVE_REAL_ERRORS, e);

                    // 诊断信息
                    if consecutive_real_errors == 1 {
                        println!("[CAPTURE] DIAGNOSTIC: First real error - possible causes:");
                        println!("[CAPTURE]   1. Windows screen capture permission denied");
                        println!("[CAPTURE]   2. Display disconnected or changed");
                        println!("[CAPTURE]   3. Another application locked the screen");
                        println!("[CAPTURE]   4. Antivirus blocking GDI/DXGI capture");
                    }

                    // 检查是否应该回退到模拟模式
                    if consecutive_real_errors >= MAX_CONSECUTIVE_REAL_ERRORS {
                        eprintln!("[CAPTURE] FATAL: Too many consecutive errors ({}), switching to simulated mode",
                            consecutive_real_errors);

                        drop(capturer);
                        start_simulated_capture_internal(sender, target_fps, target_width, target_height);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                } else {
                    // WouldBlock 是正常的 - 屏幕没有变化
                    would_block_count += 1;

                    if would_block_count >= MAX_WOULD_BLOCK_BEFORE_WARNING {
                        println!("[CAPTURE] {} WouldBlock errors since last successful frame", would_block_count);
                        println!("[CAPTURE] This is normal if screen content hasn't changed");
                        would_block_count = 0;
                    }

                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }

        last_frame_time = Instant::now();
        
        // 每30帧打印一次性能统计
        if frame_count % 30 == 0 && frame_count > 0 {
            let elapsed_total = start_time.elapsed().as_secs_f32();
            let actual_fps = frame_count as f32 / elapsed_total;
            println!("[CAPTURE] Performance: {} frames, {:.1} actual FPS (target: {})", 
                frame_count, actual_fps, target_fps);
        }
    }
    
    let total_elapsed = start_time.elapsed();
    let avg_fps = if total_elapsed.as_secs() > 0 {
        frame_count as f32 / total_elapsed.as_secs_f32()
    } else {
        0.0
    };
    println!("[CAPTURE] =========================================");
    println!("[CAPTURE] Capture stopped");
    println!("[CAPTURE] Total frames: {}", frame_count);
    println!("[CAPTURE] Empty frames: {}", empty_frame_count);
    println!("[CAPTURE] Duration: {:?}", total_elapsed);
    println!("[CAPTURE] Average FPS: {:.1}", avg_fps);
    println!("[CAPTURE] Target FPS: {}", target_fps);
    if frame_count > 0 && avg_fps < target_fps as f32 * 0.5 {
        println!("[CAPTURE] WARNING: FPS is significantly lower than target!");
        println!("[CAPTURE] Possible causes: CPU overload, capture permission issues, encoding bottleneck");
    }
    println!("[CAPTURE] =========================================");
}

/// P1 Fix: 广播模式的捕获循环 - 支持多会话共享
/// 
/// 使用tokio broadcast channel替代mpsc，允许多个接收者
fn run_capture_with_broadcast(
    mut capturer: scrap::Capturer,
    cap_width: usize,
    cap_height: usize,
    target_width: usize,
    target_height: usize,
    sender: broadcast::Sender<CapturedFrame>,
    target_fps: u32,
) {
    use scrap::Capturer;
    
    let frame_duration = Duration::from_millis(1000 / target_fps as u64);
    let needs_scaling = cap_width != target_width || cap_height != target_height;
    let start_time = Instant::now();
    let mut last_frame_time = Instant::now();
    let mut frame_count = 0u32;
    let mut consecutive_real_errors = 0u32;
    let mut would_block_count = 0u32;
    let mut empty_frame_count = 0u32;
    const MAX_CONSECUTIVE_REAL_ERRORS: u32 = 10;
    const MAX_WOULD_BLOCK_BEFORE_WARNING: u32 = 100;

    println!("[CAPTURE-BROADCAST] Starting shared screen capture");
    println!("[CAPTURE-BROADCAST] Target: {}x{} @ {} FPS", target_width, target_height, target_fps);

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last_frame_time);

        // P1 Fix: 更激进的帧率控制 - 只在严重超前时短暂sleep
        if elapsed < frame_duration {
            // 计算需要sleep的时间，但最多sleep一半，保持响应性
            let sleep_time = (frame_duration - elapsed) / 2;
            if sleep_time > Duration::from_millis(1) {
                std::thread::sleep(sleep_time);
            }
        } else if elapsed > frame_duration * 2 && frame_count % 60 == 0 {
            // 只有在严重落后且偶尔打印警告
            let behind_frames = elapsed.as_millis() / frame_duration.as_millis();
            println!("[CAPTURE-BROADCAST] Behind schedule by {} frames", behind_frames);
        }

        match capturer.frame() {
            Ok(frame) => {
                frame_count += 1;
                consecutive_real_errors = 0;
                would_block_count = 0;

                if frame.is_empty() {
                    empty_frame_count += 1;
                    if empty_frame_count % 30 == 1 {
                        println!("[CAPTURE-BROADCAST] WARNING: Empty frame received (count: {})", empty_frame_count);
                    }
                    continue;
                }

                let frame_data = if needs_scaling {
                    let scaled = resize_bgra(&frame, cap_width, cap_height, target_width, target_height);
                    CapturedFrame {
                        width: target_width,
                        height: target_height,
                        data: scaled,
                        timestamp: Instant::now(),
                    }
                } else {
                    CapturedFrame {
                        width: target_width,
                        height: target_height,
                        data: frame.to_vec(),
                        timestamp: Instant::now(),
                    }
                };

                // 使用broadcast发送，失败表示没有接收者
                if sender.send(frame_data).is_err() {
                    println!("[CAPTURE-BROADCAST] No active receivers, stopping capture");
                    break;
                }
            }
            Err(e) => {
                if e.kind() != ErrorKind::WouldBlock {
                    consecutive_real_errors += 1;
                    eprintln!("[CAPTURE-BROADCAST] REAL ERROR (consecutive: {}/{}): {}",
                        consecutive_real_errors, MAX_CONSECUTIVE_REAL_ERRORS, e);

                    if consecutive_real_errors >= MAX_CONSECUTIVE_REAL_ERRORS {
                        eprintln!("[CAPTURE-BROADCAST] Too many errors, stopping");
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                } else {
                    would_block_count += 1;
                    if would_block_count >= MAX_WOULD_BLOCK_BEFORE_WARNING {
                        println!("[CAPTURE-BROADCAST] {} WouldBlock errors since last frame", would_block_count);
                        would_block_count = 0;
                    }
                    // P1 Fix: 减少WouldBlock时的sleep时间，提高捕获频率
                    std::thread::sleep(Duration::from_micros(500));
                }
            }
        }

        last_frame_time = Instant::now();
        
        if frame_count % 30 == 0 && frame_count > 0 {
            let elapsed_total = start_time.elapsed().as_secs_f32();
            let actual_fps = frame_count as f32 / elapsed_total;
            println!("[CAPTURE-BROADCAST] Performance: {} frames, {:.1} FPS", frame_count, actual_fps);
        }
    }
    
    println!("[CAPTURE-BROADCAST] Capture stopped. Total frames: {}", frame_count);
}

/// P1 Fix: 模拟捕获广播模式
fn run_simulated_broadcast(sender: broadcast::Sender<CapturedFrame>, target_fps: u32, width: usize, height: usize) {
    let frame_duration = Duration::from_millis(1000 / target_fps as u64);
    
    std::thread::spawn(move || {
        let mut frame_count: u64 = 0;
        let start_time = Instant::now();
        
        println!("[SIMULATED-BROADCAST] Starting shared simulated capture: {}x{} @ {} FPS", width, height, target_fps);
        
        loop {
            let now = Instant::now();
            let frame_data = generate_test_pattern(width, height, frame_count);
            
            let frame = CapturedFrame {
                width,
                height,
                data: frame_data,
                timestamp: Instant::now(),
            };

            if sender.send(frame).is_err() {
                println!("[SIMULATED-BROADCAST] No active receivers, stopping");
                break;
            }

            frame_count += 1;
            
            if frame_count % 30 == 0 {
                println!("[SIMULATED-BROADCAST] Generated {} frames", frame_count);
            }

            let elapsed = now.elapsed();
            if elapsed < frame_duration {
                std::thread::sleep(frame_duration - elapsed);
            }
        }
        
        println!("[SIMULATED-BROADCAST] Stopped. Total frames: {} in {:?}", 
            frame_count, start_time.elapsed());
    });
}

/// P1 Fix: 后台模拟捕获广播（用于start_capture的async版本）
fn run_simulated_broadcast_background(sender: broadcast::Sender<CapturedFrame>, target_fps: u32, width: usize, height: usize) {
    run_simulated_broadcast(sender, target_fps, width, height);
}

/// 独立的模拟捕获函数（用于回退）
fn start_simulated_capture_internal(sender: mpsc::Sender<CapturedFrame>, target_fps: u32, width: usize, height: usize) {
    let frame_duration = Duration::from_millis(1000 / target_fps as u64);
    
    std::thread::spawn(move || {
        let mut frame_count: u64 = 0;
        let start_time = Instant::now();
        
        println!("[CAPTURE] Starting simulated capture: {}x{} @ {} FPS", width, height, target_fps);
        
        loop {
            let now = Instant::now();
            
            // 生成测试图案 - 移动的彩色条纹
            let frame_data = generate_test_pattern(width, height, frame_count);
            
            let frame = CapturedFrame {
                width,
                height,
                data: frame_data,
                timestamp: Instant::now(),
            };

            if sender.try_send(frame).is_err() {
                // 通道已满或关闭，退出
                break;
            }

            frame_count += 1;
            
            // 每30帧打印一次日志
            if frame_count % 30 == 0 {
                println!("[CAPTURE] Simulated: Generated {} frames", frame_count);
            }

            // 控制帧率
            let elapsed = now.elapsed();
            if elapsed < frame_duration {
                std::thread::sleep(frame_duration - elapsed);
            }
        }
        
        println!("[CAPTURE] Simulated capture stopped. Total frames: {} in {:?}", 
            frame_count, start_time.elapsed());
    });
}

impl ScreenCapture {
    pub fn new() -> anyhow::Result<Self> {
        // 默认使用真实屏幕捕获
        // 设置 FORCE_SIMULATED_CAPTURE=true 可强制使用模拟模式（用于无显示器环境测试）
        let force_simulated = std::env::var("FORCE_SIMULATED_CAPTURE").unwrap_or_else(|_| "false".to_string()) == "true";
        
        if force_simulated {
            println!("[CAPTURE] Forced simulated capture mode (set FORCE_SIMULATED_CAPTURE=false to use real capture)");
            let width = 1920;
            let height = 1080;
            println!("Simulated screen capture: {}x{}", width, height);
            
            return Ok(Self {
                width,
                height,
                capture_width: width,
                capture_height: height,
                mode: CaptureMode::Simulated { width, height },
            });
        }
        
        // 获取主显示器信息
        match Self::get_display_size() {
            Ok((capture_width, capture_height)) => {
                // 限制分辨率以适配编码器
                let (width, height) = Self::limit_resolution(capture_width, capture_height);

                println!("Screen capture initialized: {}x{} (original: {}x{})", 
                    width, height, capture_width, capture_height);

                Ok(Self { 
                    width, 
                    height, 
                    capture_width, 
                    capture_height,
                    mode: CaptureMode::Real,
                })
            }
            Err(e) => {
                // 无显示器环境，使用模拟模式
                println!("No display detected ({}), using simulated capture mode", e);
                let width = 1920;
                let height = 1080;
                println!("Simulated screen capture: {}x{}", width, height);
                
                Ok(Self {
                    width,
                    height,
                    capture_width: width,
                    capture_height: height,
                    mode: CaptureMode::Simulated { width, height },
                })
            }
        }
    }
    
    /// 限制分辨率在合理范围内
    ///
    /// 策略：
    /// 1. 默认使用全分辨率1920x1080以获得最佳画质
    /// 2. 可通过环境变量 CAPTURE_RES_720P=1 降低到720p以节省带宽
    /// 3. 确保宽高为偶数（I420格式要求）
    fn limit_resolution(orig_width: usize, orig_height: usize) -> (usize, usize) {
        // 检查是否降低到720p
        let use_720p = std::env::var("CAPTURE_RES_720P")
            .unwrap_or_else(|_| "false".to_string()) == "true";

        // 确保宽高都是偶数（I420格式要求）
        let orig_width = orig_width & !1;
        let orig_height = orig_height & !1;

        // 确定目标限制
        let (target_width, target_height) = if use_720p {
            println!("[CAPTURE] 720p mode enabled via CAPTURE_RES_720P=1");
            (1280, 720)
        } else {
            (MAX_ENCODER_WIDTH, MAX_ENCODER_HEIGHT)
        };

        // 如果原始分辨率已经符合要求，直接使用
        if orig_width <= target_width && orig_height <= target_height {
            println!("[CAPTURE] Using native resolution: {}x{}", orig_width, orig_height);
            return (orig_width, orig_height);
        }

        // 计算缩放比例
        let scale_w = target_width as f32 / orig_width as f32;
        let scale_h = target_height as f32 / orig_height as f32;
        let scale = scale_w.min(scale_h);

        let new_width = ((orig_width as f32 * scale) as usize) & !1;
        let new_height = ((orig_height as f32 * scale) as usize) & !1;

        println!("[CAPTURE] Resolution {}x{} scaled to {}x{} (720p_mode={})",
            orig_width, orig_height, new_width, new_height, use_720p);

        (new_width, new_height)
    }

    fn get_display_size() -> anyhow::Result<(usize, usize)> {
        // 使用 scrap 获取显示器信息
        use scrap::Display;
        
        // Display::primary() 返回 Result<Display, Error>
        match Display::primary() {
            Ok(display) => Ok((display.width(), display.height())),
            Err(e) => Err(anyhow::anyhow!("Failed to get primary display: {}", e)),
        }
    }

    /// 获取屏幕尺寸
    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// 启动捕获循环
    /// 
    /// P1 Fix: 使用单例模式，所有会话共享同一个捕获器
    pub async fn start_capture(&self, sender: mpsc::Sender<CapturedFrame>, target_fps: u32) {
        let session_id = SESSION_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        println!("[CAPTURE] Session {} starting capture (mode: {:?})", session_id, self.mode);

        let state = get_global_capture_state();
        let mut state_guard = state.lock().await;

        match &self.mode {
            CaptureMode::Real => {
                // 检查是否已有活跃的捕获器
                if let Some(ref broadcast_sender) = state_guard.sender {
                    println!("[CAPTURE] Reusing existing global capturer for session {}", session_id);
                    // 复用现有捕获器，订阅广播
                    let mut receiver = broadcast_sender.subscribe();
                    drop(state_guard);
                    
                    // 转发广播到会话的mpsc通道
                    tokio::spawn(async move {
                        while let Ok(frame) = receiver.recv().await {
                            if sender.try_send(frame).is_err() {
                                break;
                            }
                        }
                        println!("[CAPTURE] Session {} broadcast receiver stopped", session_id);
                    });
                    return;
                }

                // 没有现有捕获器，创建新的
                println!("[CAPTURE] Creating new global capturer for session {}", session_id);
                let (broadcast_tx, _) = broadcast::channel::<CapturedFrame>(100);
                state_guard.sender = Some(broadcast_tx.clone());
                state_guard.config = Some(CaptureConfig {
                    width: self.width,
                    height: self.height,
                    capture_width: self.capture_width,
                    capture_height: self.capture_height,
                    target_fps,
                    mode: self.mode.clone(),
                });

                // 创建订阅者用于当前会话
                let mut receiver = broadcast_tx.subscribe();
                
                // 启动捕获线程
                let target_width = self.width;
                let target_height = self.height;
                let capture_width = self.capture_width;
                let capture_height = self.capture_height;
                
                let handle = std::thread::spawn(move || {
                    use scrap::{Capturer, Display};
                    
                    let mut retry_count = 0;
                    let max_retries = 5;
                    
                    // 尝试创建真实捕获器
                    let capturer_result = loop {
                        let display = match Display::primary() {
                            Ok(d) => d,
                            Err(e) => {
                                eprintln!("[CAPTURE] No primary display found: {}", e);
                                break None;
                            }
                        };
                        
                        let cw = display.width();
                        let ch = display.height();
                        
                        match Capturer::new(display) {
                            Ok(c) => {
                                println!("[CAPTURE] Capturer created successfully: {}x{}", cw, ch);
                                break Some((c, cw, ch));
                            }
                            Err(e) => {
                                eprintln!("[CAPTURE] Failed to create capturer (attempt {}): {}", retry_count + 1, e);
                                if retry_count >= max_retries {
                                    eprintln!("[CAPTURE] Max retries reached, falling back to simulated mode");
                                    break None;
                                }
                                retry_count += 1;
                                std::thread::sleep(Duration::from_millis(200));
                            }
                        }
                    };
                    
                    // 如果真实捕获失败，启动模拟模式
                    let Some((mut capturer, cap_width, cap_height)) = capturer_result else {
                        eprintln!("[CAPTURE] Real capture unavailable, switching to simulated mode");
                        // 创建模拟捕获广播
                        run_simulated_broadcast(broadcast_tx, target_fps, target_width, target_height);
                        return;
                    };
                    
                    // 真实捕获成功，开始捕获循环（广播模式）
                    run_capture_with_broadcast(
                        capturer, 
                        cap_width, 
                        cap_height,
                        target_width, 
                        target_height, 
                        broadcast_tx, 
                        target_fps
                    );
                });
                
                state_guard.capture_handle = Some(handle);
                drop(state_guard);

                // 转发广播到当前会话
                tokio::spawn(async move {
                    while let Ok(frame) = receiver.recv().await {
                        if sender.try_send(frame).is_err() {
                            break;
                        }
                    }
                    println!("[CAPTURE] Session {} broadcast receiver stopped", session_id);
                });
            }
            CaptureMode::Simulated { width, height } => {
                // 检查是否已有广播通道
                if let Some(ref broadcast_sender) = state_guard.sender {
                    println!("[CAPTURE] Reusing existing simulated capturer for session {}", session_id);
                    let mut receiver = broadcast_sender.subscribe();
                    drop(state_guard);
                    
                    tokio::spawn(async move {
                        while let Ok(frame) = receiver.recv().await {
                            if sender.try_send(frame).is_err() {
                                break;
                            }
                        }
                        println!("[CAPTURE] Session {} simulated broadcast receiver stopped", session_id);
                    });
                } else {
                    // 创建新的模拟广播
                    let (broadcast_tx, _) = broadcast::channel::<CapturedFrame>(100);
                    state_guard.sender = Some(broadcast_tx.clone());
                    drop(state_guard);
                    
                    let mut receiver = broadcast_tx.subscribe();
                    run_simulated_broadcast_background(broadcast_tx, target_fps, *width, *height);
                    
                    tokio::spawn(async move {
                        while let Ok(frame) = receiver.recv().await {
                            if sender.try_send(frame).is_err() {
                                break;
                            }
                        }
                        println!("[CAPTURE] Session {} simulated receiver stopped", session_id);
                    });
                }
            }
        }
    }

    /// 真实屏幕捕获
    fn start_real_capture(&self, sender: mpsc::Sender<CapturedFrame>, target_fps: u32) {
        let target_width = self.width;
        let target_height = self.height;
        let frame_duration = Duration::from_millis(1000 / target_fps as u64);
        let needs_scaling = self.capture_width != self.width || self.capture_height != self.height;

        // 在独立线程中运行捕获
        std::thread::spawn(move || {
            use scrap::{Capturer, Display};
            
            // 重试创建捕获器
            let mut retry_count = 0;
            let max_retries = 5;
            
            let (mut capturer, capture_width, capture_height) = loop {
                // 创建捕获器
                let display = match Display::primary() {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("[CAPTURE] No primary display found: {}", e);
                        if retry_count >= max_retries {
                            eprintln!("[CAPTURE] Max retries reached, giving up");
                            return;
                        }
                        retry_count += 1;
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                };
                
                let capture_width = display.width();
                let capture_height = display.height();
                
                match Capturer::new(display) {
                    Ok(c) => {
                        println!("[CAPTURE] Capturer created successfully: {}x{}", capture_width, capture_height);
                        break (c, capture_width, capture_height);
                    }
                    Err(e) => {
                        eprintln!("[CAPTURE] Failed to create capturer (attempt {}): {}", retry_count + 1, e);
                        if retry_count >= max_retries {
                            eprintln!("[CAPTURE] Max retries reached, giving up");
                            return;
                        }
                        retry_count += 1;
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            };

            let mut last_frame_time = Instant::now();
            let mut frame_count = 0u32;

            loop {
                let now = Instant::now();
                let elapsed = now.duration_since(last_frame_time);

                if elapsed < frame_duration {
                    std::thread::sleep(frame_duration - elapsed);
                }

                // 捕获帧
                match capturer.frame() {
                    Ok(frame) => {
                        frame_count += 1;
                        
                        // 如果需要缩放，进行缩放处理
                        let frame_data = if needs_scaling {
                            let scaled = resize_bgra(&frame, capture_width, capture_height, target_width, target_height);
                            CapturedFrame {
                                width: target_width,
                                height: target_height,
                                data: scaled,
                                timestamp: Instant::now(),
                            }
                        } else {
                            CapturedFrame {
                                width: target_width,
                                height: target_height,
                                data: frame.to_vec(),
                                timestamp: Instant::now(),
                            }
                        };

                        // 使用阻塞发送，如果通道关闭才退出
                        if let Err(e) = sender.blocking_send(frame_data) {
                            eprintln!("[CAPTURE] Failed to send frame: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        if e.kind() != ErrorKind::WouldBlock {
                            eprintln!("Capture error: {}", e);
                            break;
                        }
                        // WouldBlock 意味着需要重试
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }

                last_frame_time = Instant::now();
            }
        });
    }

    /// 模拟屏幕捕获（生成测试图案）
    fn start_simulated_capture(&self, sender: mpsc::Sender<CapturedFrame>, target_fps: u32, width: usize, height: usize) {
        start_simulated_capture_internal(sender, target_fps, width, height);
    }
}

/// 生成测试图案（BGRA格式）
fn generate_test_pattern(width: usize, height: usize, frame_count: u64) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];
    
    // 动画参数
    let offset = (frame_count % 120) as f32;
    
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            
            // 创建移动的渐变图案
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;
            
            // 基于位置的彩虹效果
            let r = ((fx * 255.0 + offset) % 255.0) as u8;
            let g = ((fy * 255.0 + offset * 2.0) % 255.0) as u8;
            let b = (((fx + fy) * 127.5 + offset * 3.0) % 255.0) as u8;
            
            // BGRA格式
            data[idx] = b;     // B
            data[idx + 1] = g; // G
            data[idx + 2] = r; // R
            data[idx + 3] = 255; // A (不透明)
        }
    }
    
    // 添加"REMOTE DESKTOP"文字效果（简单的白色条纹）
    let stripe_y = ((frame_count as usize * 5) % height);
    for x in 0..width {
        let idx = (stripe_y * width + x) * 4;
        if idx + 3 < data.len() {
            data[idx] = 255;     // B
            data[idx + 1] = 255; // G
            data[idx + 2] = 255; // R
        }
    }
    
    data
}

/// 视频编码器 trait
#[async_trait::async_trait]
pub trait VideoEncoder: Send + Sync {
    async fn encode(&mut self, frame: &CapturedFrame) -> anyhow::Result<Vec<u8>>;
    fn set_bitrate(&mut self, bitrate: u32);
}

/// 简单的帧编码器 - Phase 1 简化版
pub struct SimpleFrameEncoder {
    target_bitrate: u32,
    quality: u8, // 0-100
}

impl SimpleFrameEncoder {
    pub fn new(bitrate: u32) -> Self {
        Self {
            target_bitrate: bitrate,
            quality: 80,
        }
    }

    /// 将 BGRA 帧转换为更紧凑的格式
    pub fn encode_frame(&mut self, frame: &CapturedFrame) -> anyhow::Result<Vec<u8>> {
        // Phase 1 简化：返回 BGRA 数据带头部
        // 格式: [width: u32][height: u32][timestamp: u64][BGRA data...]
        let mut result = Vec::with_capacity(16 + frame.data.len());
        result.extend_from_slice(&(frame.width as u32).to_le_bytes());
        result.extend_from_slice(&(frame.height as u32).to_le_bytes());
        result.extend_from_slice(&(frame.timestamp.elapsed().as_millis() as u64).to_le_bytes());
        result.extend_from_slice(&frame.data);
        
        Ok(result)
    }
}

#[async_trait::async_trait]
impl VideoEncoder for SimpleFrameEncoder {
    async fn encode(&mut self, frame: &CapturedFrame) -> anyhow::Result<Vec<u8>> {
        self.encode_frame(frame)
    }

    fn set_bitrate(&mut self, bitrate: u32) {
        self.target_bitrate = bitrate;
        self.quality = ((bitrate / 1000).min(100)).max(10) as u8;
    }
}

/// H.264 视频编码器 (使用 openh264，支持1080p)
pub struct Vp8Encoder {
    encoder: openh264::encoder::Encoder,
    width: usize,
    height: usize,
    frame_count: u32,
}

impl Vp8Encoder {
    /// 创建新的 H.264 编码器
    /// 
    /// # Arguments
    /// * `width` - 视频宽度
    /// * `height` - 视频高度
    /// * `bitrate` - 目标比特率 (bps)
    pub fn new(width: usize, height: usize, target_bps: u32) -> anyhow::Result<Self> {
        use openh264::encoder::{Encoder, EncoderConfig, BitRate, Profile, Level, UsageType};
        use openh264::OpenH264API;
        
        println!("[ENCODER] Creating H.264 encoder: {}x{} @ {} bps", width, height, target_bps);
        
        if width == 0 || height == 0 {
            return Err(anyhow::anyhow!("Invalid dimensions: {}x{}", width, height));
        }
        
        if width % 2 != 0 || height % 2 != 0 {
            eprintln!("[ENCODER] WARNING: Dimensions should be even for I420 format! {}x{}", width, height);
        }
        
        let level = Level::Level_4_0;

        // P1 Fix: 优化编码器配置以提升帧率
        // - 禁用帧跳过，确保每帧都被编码
        // - 使用更高的码率保证质量
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(target_bps))
            .max_frame_rate(openh264::encoder::FrameRate::from_hz(30.0))
            .profile(Profile::Baseline)
            .level(level)
            .usage_type(UsageType::CameraVideoRealTime)
            .skip_frames(false);  // 禁用帧跳过，保证帧率
        
        let api = OpenH264API::from_source();
        let encoder = Encoder::with_api_config(api, config)?;
        
        println!("[ENCODER] H.264 encoder created successfully");
        println!("[ENCODER]   Profile: Baseline");
        println!("[ENCODER]   Level: 4.0");
        println!("[ENCODER]   Bitrate: {} bps", target_bps);
        println!("[ENCODER]   Max FPS: 30");
        println!("[ENCODER]   Dimensions: {}x{}", width, height);
        
        Ok(Self {
            encoder,
            width,
            height,
            frame_count: 0,
        })
    }
    
    /// 检查是否需要生成关键帧
    fn maybe_force_keyframe(&mut self) {
        self.frame_count += 1;
        // 首帧强制关键帧，之后每30帧一个关键帧
        if self.frame_count == 1 || self.frame_count % 30 == 1 {
            println!("[ENCODER] Requesting IDR keyframe #{} (frame {})", 
                (self.frame_count - 1) / 30 + 1, self.frame_count);
            self.encoder.force_intra_frame();
        }
    }
    
    /// 编码一帧 I420 数据
    /// 
    /// # Arguments
    /// * `i420_data` - I420 (YUV420P) 格式的帧数据
    /// 
    /// # Returns
    /// 编码后的 H.264 数据
    pub fn encode_i420(&mut self, i420_data: &[u8]) -> anyhow::Result<Vec<u8>> {
        use openh264::formats::YUVSource;
        
        let y_size = self.width * self.height;
        let uv_size = y_size / 4;
        
        if i420_data.len() != y_size + 2 * uv_size {
            return Err(anyhow::anyhow!(
                "Invalid I420 data size: expected {}, got {}",
                y_size + 2 * uv_size,
                i420_data.len()
            ));
        }
        
        // 检查是否需要强制生成关键帧
        self.maybe_force_keyframe();
        
        // 将 I420 数据拆分为 Y/U/V 平面
        let y_plane = &i420_data[0..y_size];
        let u_plane = &i420_data[y_size..y_size + uv_size];
        let v_plane = &i420_data[y_size + uv_size..];
        
        // 创建 YUV420P 源
        let yuv_source = Yuv420PSource {
            y: y_plane,
            u: u_plane,
            v: v_plane,
            width: self.width,
            height: self.height,
        };
        
        let bitstream = self.encoder.encode(&yuv_source)?;
        let mut data = bitstream.to_vec();
        
        // P0 Fix: 处理编码器返回空数据的情况 - 强制关键帧重试
        if data.is_empty() {
            eprintln!("[ENCODER] WARNING: Encoder returned empty data for frame #{}, retrying with forced keyframe...", self.frame_count);
            self.encoder.force_intra_frame();
            let retry_bitstream = self.encoder.encode(&yuv_source)?;
            data = retry_bitstream.to_vec();
            
            if !data.is_empty() {
                println!("[ENCODER] Retry successful for frame #{}, got {} bytes", self.frame_count, data.len());
            } else {
                eprintln!("[ENCODER] ERROR: Retry also failed for frame #{}, returning empty data", self.frame_count);
            }
        }
        
        if self.frame_count % 30 == 1 && !data.is_empty() {
            let is_keyframe = data.len() > 4 && (
                (data[4] & 0x1F) == 5 || // IDR slice
                (data[4] & 0x1F) == 7 || // SPS
                (data[4] & 0x1F) == 8    // PPS
            );
            println!("[ENCODER] Encoded keyframe #{}: {} bytes, nal_type={:#04x}, is_keyframe={}", 
                (self.frame_count - 1) / 30 + 1, data.len(),
                if data.len() > 4 { data[4] } else { 0 },
                is_keyframe);
        }
        
        Ok(data)
    }
    
    /// 从 BGRA 帧直接编码
    ///
    /// # Arguments
    /// * `frame` - BGRA 格式的捕获帧
    ///
    /// # Returns
    /// 编码后的 H.264 数据
    pub fn encode_frame(&mut self, frame: &CapturedFrame) -> anyhow::Result<Vec<u8>> {
        let total_start = Instant::now();
        
        // 如果帧尺寸与编码器不匹配，进行缩放
        let frame_data = if frame.width != self.width || frame.height != self.height {
            resize_bgra(&frame.data, frame.width, frame.height, self.width, self.height)
        } else {
            frame.data.clone()
        };
        
        // 性能分析：BGRA转I420
        let convert_start = Instant::now();
        let i420 = bgra_to_i420(&frame_data, self.width, self.height);
        let convert_time = convert_start.elapsed();
        
        // 性能分析：H.264编码
        let encode_start = Instant::now();
        let result = self.encode_i420(&i420);
        let encode_time = encode_start.elapsed();
        
        let total_time = total_start.elapsed();

        match &result {
            Ok(data) => {
                let is_keyframe = data.len() > 4 && (
                    (data[4] & 0x1F) == 5 || // IDR slice
                    (data[4] & 0x1F) == 7 || // SPS
                    (data[4] & 0x1F) == 8    // PPS
                );
                
                if data.is_empty() {
                    eprintln!("[ENCODE] Frame #{} FAILED: convert={:?}, encode={:?}, total={:?}",
                        self.frame_count, convert_time, encode_time, total_time);
                } else {
                    // 性能分析输出：显示各环节耗时
                    println!("[ENCODE-PERF] Frame #{}: convert={:?}, encode={:?}, total={:?} | {} bytes -> {} bytes, keyframe={}",
                        self.frame_count, convert_time, encode_time, total_time,
                        frame.data.len(), data.len(), is_keyframe);
                    
                    // 警告：如果转换或编码耗时过长
                    if convert_time > Duration::from_millis(16) {
                        eprintln!("[ENCODE-PERF] WARNING: Color conversion too slow! {:?} > 16ms", convert_time);
                    }
                    if encode_time > Duration::from_millis(16) {
                        eprintln!("[ENCODE-PERF] WARNING: H.264 encoding too slow! {:?} > 16ms", encode_time);
                    }
                }
            }
            Err(e) => {
                eprintln!("[ENCODE] Frame encoding FAILED: {}", e);
            }
        }

        result
    }
    
    /// 获取视频尺寸
    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    
    /// 获取视频宽度
    pub fn width(&self) -> usize {
        self.width
    }
    
    /// 获取视频高度
    pub fn height(&self) -> usize {
        self.height
    }
}

/// YUV420P 源结构，用于 openh264 编码器
struct Yuv420PSource<'a> {
    y: &'a [u8],
    u: &'a [u8],
    v: &'a [u8],
    width: usize,
    height: usize,
}

impl<'a> openh264::formats::YUVSource for Yuv420PSource<'a> {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    
    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }
    
    fn y(&self) -> &[u8] {
        self.y
    }
    
    fn u(&self) -> &[u8] {
        self.u
    }
    
    fn v(&self) -> &[u8] {
        self.v
    }
}

/// 创建帧处理管道
pub async fn create_frame_pipeline(
    target_fps: u32,
    bitrate: u32
) -> anyhow::Result<(ScreenCapture, mpsc::Receiver<Vec<u8>>)> {
    let (frame_sender, mut frame_receiver) = mpsc::channel::<CapturedFrame>(100);
    let (output_sender, output_receiver) = mpsc::channel::<Vec<u8>>(100);

    let capture = ScreenCapture::new()?;
    capture.start_capture(frame_sender, target_fps).await;

    // 启动处理循环
    tokio::spawn(async move {
        let mut encoder = SimpleFrameEncoder::new(bitrate);
        
        while let Some(frame) = frame_receiver.recv().await {
            match encoder.encode(&frame).await {
                Ok(encoded) => {
                    if output_sender.send(encoded).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Encoding error: {}", e);
                }
            }
        }
    });

    Ok((capture, output_receiver))
}
