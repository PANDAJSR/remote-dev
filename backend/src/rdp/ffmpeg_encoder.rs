use anyhow::{anyhow, Result};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub struct FFmpegEncoder {
    width: usize,
    height: usize,
    bitrate: u32,
    frame_count: u64,
    encoder_type: String,
    ffmpeg_process: Option<Child>,
    output_receiver: Option<mpsc::Receiver<Vec<u8>>>,
    consecutive_errors: u32,
}

impl FFmpegEncoder {
    pub fn new(width: usize, height: usize, bitrate: u32) -> Result<Self> {
        println!(
            "[FFMPEG-PERSISTENT] Creating encoder: {}x{} @ {} bps",
            width, height, bitrate
        );

        let encoder_type = Self::detect_best_encoder();
        println!("[FFMPEG-PERSISTENT] Using encoder: {}", encoder_type);

        let mut encoder = Self {
            width,
            height,
            bitrate,
            frame_count: 0,
            encoder_type,
            ffmpeg_process: None,
            output_receiver: None,
            consecutive_errors: 0,
        };

        encoder.start_ffmpeg_process()?;
        Ok(encoder)
    }

    fn start_ffmpeg_process(&mut self) -> Result<()> {
        println!("[FFMPEG-PERSISTENT] Starting FFmpeg process...");

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("info") // 改为 info 减少日志输出
            .arg("-fflags")
            .arg("nobuffer")
            .arg("-flags")
            .arg("low_delay")
            .arg("-thread_queue_size")
            .arg("1024") // 增加输入缓冲区
            .arg("-f")
            .arg("rawvideo")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-s")
            .arg(format!("{}x{}", self.width, self.height))
            .arg("-r")
            .arg("30")
            .arg("-i")
            .arg("-");

        match self.encoder_type.as_str() {
            "h264_nvenc" => {
                // NVENC H.264编码器配置 - 优化低延迟和快速启动
                cmd.arg("-c:v")
                    .arg("h264_nvenc")
                    .arg("-preset")
                    .arg("p1") // 最快的预设
                    .arg("-tune")
                    .arg("ull") // 超低延迟模式
                    .arg("-profile:v")
                    .arg("baseline")
                    .arg("-level")
                    .arg("4.0")
                    .arg("-rc")
                    .arg("cbr")
                    .arg("-b:v")
                    .arg(format!("{}", self.bitrate))
                    .arg("-maxrate:v")
                    .arg(format!("{}", self.bitrate))
                    .arg("-bufsize:v")
                    .arg(format!("{}", self.bitrate))
                    .arg("-zerolatency")
                    .arg("1")
                    .arg("-bf")
                    .arg("0")
                    .arg("-refs")
                    .arg("1")
                    .arg("-g")
                    .arg("30")
                    .arg("-keyint_min")
                    .arg("30");
            }
            _ => {
                // libx264软件编码器配置
                cmd.arg("-c:v")
                    .arg("libx264")
                    .arg("-preset")
                    .arg("ultrafast")
                    .arg("-tune")
                    .arg("zerolatency")
                    .arg("-profile:v")
                    .arg("baseline") // WebRTC要求baseline profile
                    .arg("-level")
                    .arg("4.0")
                    .arg("-bf")
                    .arg("0") // 禁用B帧
                    .arg("-refs")
                    .arg("1");
            }
        }

        cmd.arg("-sc_threshold")
            .arg("0") // 禁用场景切换检测
            .arg("-flush_packets")
            .arg("1") // 强制立即刷新每个包
            .arg("-an") // 禁用音频
            .arg("-sn") // 禁用字幕
            .arg("-f")
            .arg("h264") // 输出原始H264 Annex B格式
            .arg("-");

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn ffmpeg: {}", e))?;

        println!("[FFMPEG-PERSISTENT] FFmpeg started (PID: {:?})", child.id());

        // 等待一小段时间确保 FFmpeg 启动
        std::thread::sleep(Duration::from_millis(100));

        // 检查进程是否还在运行
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!(
                "[FFMPEG-PERSISTENT] FFmpeg exited immediately with status: {:?}",
                status
            );
            return Err(anyhow!("FFmpeg exited immediately"));
        }

        println!("[FFMPEG-PERSISTENT] FFmpeg process is running");

        // 创建通道用于接收编码后的数据
        let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>();
        self.output_receiver = Some(output_rx);

        // 启动输出读取线程 - 使用NAL起始码检测来正确分割H264帧
        let stdout = child.stdout.take().expect("Failed to get stdout");
        thread::spawn(move || {
            let mut stdout = stdout;
            let mut buffer = [0u8; 65536];
            let mut buffered_data = Vec::new();
            let mut total_read = 0usize;
            let mut frame_count = 0usize;
            let start_time = Instant::now();

            println!("[FFMPEG-OUTPUT] Output thread started with Access Unit framing");

            // H.264 NAL起始码检测函数 - 返回起始码位置和长度
            fn find_nal_start_code(data: &[u8], start: usize) -> Option<(usize, usize)> {
                for i in start..data.len().saturating_sub(3) {
                    // 检查4字节起始码: 00 00 00 01
                    if data[i] == 0x00
                        && data[i + 1] == 0x00
                        && data[i + 2] == 0x00
                        && data[i + 3] == 0x01
                    {
                        return Some((i, 4));
                    }
                    // 检查3字节起始码: 00 00 01
                    if data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x01 {
                        return Some((i, 3));
                    }
                }
                None
            }

            // 获取NAL类型
            fn get_nal_type(nal_unit: &[u8]) -> (u8, String) {
                if nal_unit.len() < 5 {
                    return (0, "too_short".to_string());
                }

                // 检测起始码类型
                let nal_type_pos = if nal_unit[0] == 0x00 && nal_unit[1] == 0x00 {
                    if nal_unit[2] == 0x00 && nal_unit[3] == 0x01 {
                        4 // 4字节起始码: 00 00 00 01
                    } else if nal_unit[2] == 0x01 {
                        3 // 3字节起始码: 00 00 01
                    } else {
                        return (0, "invalid_start_code".to_string());
                    }
                } else {
                    return (0, "no_start_code".to_string());
                };

                if nal_type_pos >= nal_unit.len() {
                    return (0, "truncated".to_string());
                }

                let nal_header = nal_unit[nal_type_pos];
                let nal_type = nal_header & 0x1F;

                let type_str = match nal_type {
                    0 => "unspecified",
                    1 => "P-slice",
                    5 => "IDR",
                    6 => "SEI",
                    7 => "SPS",
                    8 => "PPS",
                    9 => "AUD",
                    10 => "end_of_seq",
                    11 => "end_of_stream",
                    12 => "filler",
                    13..=23 => "reserved",
                    24 => "STAP-A",
                    25 => "STAP-B",
                    26 => "MTAP16",
                    27 => "MTAP24",
                    28 => "FU-A",
                    29 => "FU-B",
                    30..=31 => "undefined",
                    _ => "unknown",
                };

                (nal_type, type_str.to_string())
            }

            // 收集访问单元（Access Unit）的所有NAL单元
            let mut current_au = Vec::new(); // 当前访问单元
            let mut au_nal_types = Vec::new(); // 当前AU中的NAL类型

            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => {
                        println!(
                            "[FFMPEG-OUTPUT] EOF received, total bytes read: {}, frames sent: {}",
                            total_read, frame_count
                        );
                        // 发送剩余的访问单元
                        if !current_au.is_empty() {
                            println!(
                                "[FFMPEG-OUTPUT] Sending final AU: {} bytes, NALs: {:?}",
                                current_au.len(),
                                au_nal_types
                            );
                            let _ = output_tx.send(current_au);
                            frame_count += 1;
                        }
                        break;
                    }
                    Ok(n) => {
                        total_read += n;
                        buffered_data.extend_from_slice(&buffer[..n]);

                        // 使用NAL起始码分割H.264帧
                        let mut nal_positions = Vec::new();
                        let mut search_start = 0;

                        while let Some((pos, len)) =
                            find_nal_start_code(&buffered_data, search_start)
                        {
                            nal_positions.push((pos, len));
                            search_start = pos + len;
                        }

                        // 如果没有找到起始码，保留数据继续读取
                        if nal_positions.is_empty() {
                            // 保留最后6字节以防它们是起始码的一部分
                            if buffered_data.len() > 6 {
                                buffered_data = buffered_data[buffered_data.len() - 6..].to_vec();
                            }
                            continue;
                        }

                        // 处理所有完整的NAL单元
                        let mut sent_au = false;
                        for i in 0..nal_positions.len() {
                            let (start_pos, _) = nal_positions[i];
                            let end_pos = if i + 1 < nal_positions.len() {
                                nal_positions[i + 1].0
                            } else {
                                // 这是最后一个NAL单元，可能不完整
                                break;
                            };

                            let nal_unit = &buffered_data[start_pos..end_pos];

                            if nal_unit.len() < 5 {
                                continue;
                            }

                            let (nal_type, type_str) = get_nal_type(nal_unit);

                            // 跳过AUD、填充等非VCL NAL
                            if nal_type == 9 || nal_type == 12 {
                                continue;
                            }

                            // 判断是否是新访问单元的开始
                            // 在H.264中，IDR帧（type=5）或P-slice（type=1）标志着新的访问单元开始
                            let is_vcl_nal = nal_type == 5 || nal_type == 1;

                            if is_vcl_nal && !current_au.is_empty() {
                                // 发送之前的访问单元
                                println!(
                                    "[FFMPEG-OUTPUT] AU[{}]: {} bytes, NAL types: {:?}",
                                    frame_count,
                                    current_au.len(),
                                    au_nal_types
                                );

                                if output_tx.send(current_au.clone()).is_err() {
                                    println!("[FFMPEG-OUTPUT] Channel closed, exiting");
                                    return;
                                }
                                frame_count += 1;
                                sent_au = true;

                                // 开始新的访问单元
                                current_au.clear();
                                au_nal_types.clear();
                            }

                            // 将NAL单元添加到当前访问单元
                            current_au.extend_from_slice(nal_unit);
                            au_nal_types.push((nal_type, type_str.clone()));

                            // 如果是关键帧，立即发送（确保首帧及时送达）
                            if nal_type == 5 && !sent_au {
                                println!(
                                    "[FFMPEG-OUTPUT] AU[{}]: {} bytes (IDR immediate), NAL types: {:?}",
                                    frame_count,
                                    current_au.len(),
                                    au_nal_types
                                );
                                if output_tx.send(current_au.clone()).is_err() {
                                    println!("[FFMPEG-OUTPUT] Channel closed, exiting");
                                    return;
                                }
                                frame_count += 1;
                                sent_au = true;
                                current_au.clear();
                                au_nal_types.clear();
                            }
                        }

                        // 保留从最后一个起始码开始的数据
                        if let Some(&(last_pos, _)) = nal_positions.last() {
                            buffered_data = buffered_data[last_pos..].to_vec();
                        }

                        // 定期打印状态
                        if frame_count > 0 && frame_count % 30 == 0 {
                            let elapsed = start_time.elapsed().as_secs_f32();
                            println!(
                                "[FFMPEG-OUTPUT] Stats: {} AUs, {} bytes read, {:.1} AUs/sec",
                                frame_count,
                                total_read,
                                frame_count as f32 / elapsed.max(0.001)
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("[FFMPEG-OUTPUT] Read error: {}", e);
                        break;
                    }
                }
            }
            println!(
                "[FFMPEG-OUTPUT] Output thread exited, total bytes: {}, frames: {}",
                total_read, frame_count
            );
        });

        // 启动stderr读取线程
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                println!("[FFMPEG-STDERR] Stderr thread started");
                let mut stderr = stderr;
                let mut buffer = [0u8; 1024];
                let mut has_output = false;
                loop {
                    match stderr.read(&mut buffer) {
                        Ok(0) => {
                            println!("[FFMPEG-STDERR] EOF received, had_output={}", has_output);
                            break;
                        }
                        Ok(n) => {
                            has_output = true;
                            let msg = String::from_utf8_lossy(&buffer[..n]);
                            for line in msg.lines() {
                                let trimmed = line.trim();
                                if !trimmed.is_empty() {
                                    eprintln!("[FFMPEG-LOG] {}", trimmed);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[FFMPEG-STDERR] Read error: {}", e);
                            break;
                        }
                    }
                }
                println!("[FFMPEG-STDERR] Stderr thread exited");
            });
        } else {
            eprintln!("[FFMPEG-PERSISTENT] WARNING: Failed to get stderr");
        }

        self.ffmpeg_process = Some(child);
        println!("[FFMPEG-PERSISTENT] Encoder ready");
        Ok(())
    }

    fn detect_best_encoder() -> String {
        // 允许通过环境变量强制使用特定编码器
        if let Ok(force_encoder) = std::env::var("FORCE_ENCODER") {
            println!(
                "[FFMPEG-PERSISTENT] Using forced encoder: {}",
                force_encoder
            );
            return force_encoder;
        }

        if let Ok(output) = Command::new("ffmpeg")
            .args(&["-hide_banner", "-encoders"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // 优先检查NVENC，但可以通过环境变量禁用
            if stdout.contains("h264_nvenc") && std::env::var("DISABLE_NVENC").is_err() {
                println!("[FFMPEG-PERSISTENT] NVENC encoder detected and enabled");
                return "h264_nvenc".to_string();
            }
        }
        println!("[FFMPEG-PERSISTENT] Using software encoder: libx264");
        "libx264".to_string()
    }

    pub fn encode_frame(&mut self, bgra: &[u8]) -> Result<Vec<u8>> {
        let start = Instant::now();

        // 检查进程状态
        if let Some(ref mut child) = self.ffmpeg_process {
            if let Ok(Some(_)) = child.try_wait() {
                eprintln!("[FFMPEG-PERSISTENT] FFmpeg exited, restarting...");
                self.restart_process()?;
            }
        } else {
            return Err(anyhow!("FFmpeg not running"));
        }

        // 转换为I420
        let i420 = Self::bgra_to_i420(bgra, self.width, self.height);
        let convert_time = start.elapsed();

        // 获取stdin并写入数据
        let child = self.ffmpeg_process.as_mut().unwrap();
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("Stdin not available"))?;

        println!(
            "[FFMPEG-ENCODE] Writing {} bytes to FFmpeg stdin...",
            i420.len()
        );
        stdin.write_all(&i420)?;
        stdin.flush()?;
        println!("[FFMPEG-ENCODE] Data written, waiting for output...");

        // 从通道接收编码后的数据
        let receiver = self
            .output_receiver
            .as_ref()
            .ok_or_else(|| anyhow!("Output receiver not available"))?;

        // NVENC编码器第一帧初始化可能需要较长时间（初始化GPU上下文）
        // 根据用户反馈，NVENC 首次初始化可能需要 40+ 秒
        let timeout = if self.frame_count == 0 {
            Duration::from_secs(60) // 第一帧给60秒超时
        } else {
            Duration::from_millis(2000) // 后续帧2秒超时
        };

        println!(
            "[FFMPEG-ENCODE] Waiting for encoded data with timeout {:?}...",
            timeout
        );
        let encoded_data = match receiver.recv_timeout(timeout) {
            Ok(data) => {
                println!("[FFMPEG-ENCODE] Received {} bytes from channel", data.len());
                if data.len() < 10 {
                    eprintln!(
                        "[FFMPEG-ENCODE] WARNING: Received data too small: {} bytes",
                        data.len()
                    );
                }
                data
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!(
                    "[FFMPEG-ENCODE] Timeout waiting for data from channel after {:?}",
                    timeout
                );
                self.consecutive_errors += 1;
                if self.consecutive_errors >= 3 {
                    eprintln!(
                        "[FFMPEG-ENCODE] Too many consecutive errors ({}), restarting FFmpeg",
                        self.consecutive_errors
                    );
                    self.restart_process()?;
                }
                return Err(anyhow!("Timeout waiting for encoded data"));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!(
                    "[FFMPEG-ENCODE] Channel disconnected, FFmpeg output thread may have crashed"
                );
                return Err(anyhow!("Output channel disconnected"));
            }
        };

        let total_time = start.elapsed();
        self.frame_count += 1;
        self.consecutive_errors = 0;

        if self.frame_count % 30 == 1 {
            println!(
                "[FFMPEG-PERF] Frame {}: convert={:?}, total={:?}, size={} bytes",
                self.frame_count,
                convert_time,
                total_time,
                encoded_data.len()
            );
        }

        if encoded_data.is_empty() {
            Err(anyhow!("Empty encoded data"))
        } else {
            Ok(encoded_data)
        }
    }

    fn restart_process(&mut self) -> Result<()> {
        println!("[FFMPEG-PERSISTENT] Restarting FFmpeg process...");

        // 丢弃旧的接收者（这会关闭通道）
        self.output_receiver = None;

        if let Some(mut child) = self.ffmpeg_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        self.start_ffmpeg_process()?;
        self.consecutive_errors = 0;
        println!("[FFMPEG-PERSISTENT] FFmpeg restarted");
        Ok(())
    }

    pub fn force_intra_frame(&mut self) {
        println!(
            "[FFMPEG-PERSISTENT] Keyframe requested at frame {}",
            self.frame_count
        );
    }

    fn bgra_to_i420(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
        use rayon::prelude::*;

        let y_size = width * height;
        let uv_size = y_size / 4;
        let mut i420 = vec![0u8; y_size + 2 * uv_size];

        let (y_plane, rest) = i420.split_at_mut(y_size);
        let (u_plane, v_plane) = rest.split_at_mut(uv_size);

        y_plane
            .par_chunks_mut(width)
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

        let half_w = width / 2;
        let half_h = height / 2;

        let uv_data: Vec<(usize, u8, u8)> = (0..half_h)
            .into_par_iter()
            .flat_map(|y| {
                let y2 = y * 2;
                (0..half_w)
                    .into_par_iter()
                    .map(move |x| {
                        let x2 = x * 2;
                        let idx00 = (y2 * width + x2) * 4;
                        let idx01 = idx00 + 4;
                        let idx10 = ((y2 + 1) * width + x2) * 4;
                        let idx11 = idx10 + 4;

                        let b = (bgra[idx00] as i32
                            + bgra[idx01] as i32
                            + bgra[idx10] as i32
                            + bgra[idx11] as i32)
                            >> 2;
                        let g = (bgra[idx00 + 1] as i32
                            + bgra[idx01 + 1] as i32
                            + bgra[idx10 + 1] as i32
                            + bgra[idx11 + 1] as i32)
                            >> 2;
                        let r = (bgra[idx00 + 2] as i32
                            + bgra[idx01 + 2] as i32
                            + bgra[idx10 + 2] as i32
                            + bgra[idx11 + 2] as i32)
                            >> 2;

                        let u = ((-44 * r - 87 * g + 131 * b) >> 8) + 128;
                        let v = ((131 * r - 110 * g - 21 * b) >> 8) + 128;

                        (y * half_w + x, u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
                    })
                    .collect::<Vec<_>>()
                    .into_par_iter()
            })
            .collect();

        for (idx, u_val, v_val) in uv_data {
            u_plane[idx] = u_val;
            v_plane[idx] = v_val;
        }

        i420
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn encoder_type(&self) -> &str {
        &self.encoder_type
    }
}

impl Drop for FFmpegEncoder {
    fn drop(&mut self) {
        println!(
            "[FFMPEG-PERSISTENT] Dropping encoder (total frames: {})",
            self.frame_count
        );
        if let Some(mut child) = self.ffmpeg_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

unsafe impl Send for FFmpegEncoder {}
unsafe impl Sync for FFmpegEncoder {}
