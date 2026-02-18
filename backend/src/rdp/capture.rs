use std::io::ErrorKind;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// 捕获的帧数据
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>, // BGRA 格式
    pub timestamp: Instant,
}

/// 简单的BGRA图像缩放（最近邻插值）
fn resize_bgra(src: &[u8], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dst_w * dst_h * 4];
    
    let x_ratio = src_w as f32 / dst_w as f32;
    let y_ratio = src_h as f32 / dst_h as f32;
    
    for y in 0..dst_h {
        for x in 0..dst_w {
            let src_x = (x as f32 * x_ratio) as usize;
            let src_y = (y as f32 * y_ratio) as usize;
            
            let src_idx = (src_y * src_w + src_x) * 4;
            let dst_idx = (y * dst_w + x) * 4;
            
            if src_idx + 3 < src.len() {
                dst[dst_idx] = src[src_idx];
                dst[dst_idx + 1] = src[src_idx + 1];
                dst[dst_idx + 2] = src[src_idx + 2];
                dst[dst_idx + 3] = src[src_idx + 3];
            }
        }
    }
    
    dst
}

/// 将 BGRA 转换为 I420 (YUV420P) 格式
/// 输出格式: Y平面 (width*height) + U平面 (width*height/4) + V平面 (width*height/4)
pub fn bgra_to_i420(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_size = y_size / 4;
    let mut i420 = vec![0u8; y_size + 2 * uv_size];
    
    // 使用 split_at_mut 避免多个可变借用
    let (y_plane, rest) = i420.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);
    
    let y_stride = width;
    let uv_stride = width / 2;
    
    // Simple implementation for now - can be optimized with SIMD
    for y in 0..height {
        for x in 0..width {
            let pixel_idx = (y * width + x) * 4;
            let b = bgra[pixel_idx] as f32;
            let g = bgra[pixel_idx + 1] as f32;
            let r = bgra[pixel_idx + 2] as f32;
            // Alpha (idx + 3) is ignored
            
            // Y = 0.299R + 0.587G + 0.114B
            let y_val = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
            y_plane[y * y_stride + x] = y_val;
            
            // Subsample UV for every 2x2 block
            if y % 2 == 0 && x % 2 == 0 {
                let uv_idx = (y / 2) * uv_stride + (x / 2);
                
                // Sample 2x2 block
                let mut r_sum = r;
                let mut g_sum = g;
                let mut b_sum = b;
                
                // Add right pixel if exists
                if x + 1 < width {
                    let right_idx = pixel_idx + 4;
                    r_sum += bgra[right_idx + 2] as f32;
                    g_sum += bgra[right_idx + 1] as f32;
                    b_sum += bgra[right_idx] as f32;
                }
                
                // Add bottom pixel if exists
                if y + 1 < height {
                    let bottom_idx = ((y + 1) * width + x) * 4;
                    r_sum += bgra[bottom_idx + 2] as f32;
                    g_sum += bgra[bottom_idx + 1] as f32;
                    b_sum += bgra[bottom_idx] as f32;
                }
                
                // Add bottom-right pixel if exists
                if x + 1 < width && y + 1 < height {
                    let br_idx = ((y + 1) * width + (x + 1)) * 4;
                    r_sum += bgra[br_idx + 2] as f32;
                    g_sum += bgra[br_idx + 1] as f32;
                    b_sum += bgra[br_idx] as f32;
                }
                
                // Average
                let count = if x + 1 < width && y + 1 < height { 4.0 } 
                    else if x + 1 < width || y + 1 < height { 2.0 } 
                    else { 1.0 };
                
                let r_avg = r_sum / count;
                let g_avg = g_sum / count;
                let b_avg = b_sum / count;
                
                // U = -0.169R - 0.331G + 0.499B + 128
                // V = 0.499R - 0.419G - 0.081B + 128
                let u_val = (-0.169 * r_avg - 0.331 * g_avg + 0.499 * b_avg + 128.0) as u8;
                let v_val = (0.499 * r_avg - 0.419 * g_avg - 0.081 * b_avg + 128.0) as u8;
                
                u_plane[uv_idx] = u_val;
                v_plane[uv_idx] = v_val;
            }
        }
    }
    
    i420
}

/// OpenH264 编码器最大支持的分辨率
const MAX_ENCODER_WIDTH: usize = 3840;
const MAX_ENCODER_HEIGHT: usize = 2160;

/// 屏幕捕获器
pub struct ScreenCapture {
    width: usize,
    height: usize,
    capture_width: usize,
    capture_height: usize,
}

impl ScreenCapture {
    pub fn new() -> anyhow::Result<Self> {
        // 获取主显示器信息
        let (capture_width, capture_height) = Self::get_display_size()?;
        
        // 限制分辨率以适配编码器
        let (width, height) = Self::limit_resolution(capture_width, capture_height);

        println!("Screen capture initialized: {}x{} (original: {}x{})", 
            width, height, capture_width, capture_height);

        Ok(Self { 
            width, 
            height, 
            capture_width, 
            capture_height 
        })
    }
    
    /// 限制分辨率在编码器支持范围内
    fn limit_resolution(orig_width: usize, orig_height: usize) -> (usize, usize) {
        // 确保宽高都是偶数（I420格式要求）
        let orig_width = orig_width & !1;
        let orig_height = orig_height & !1;
        
        if orig_width <= MAX_ENCODER_WIDTH && orig_height <= MAX_ENCODER_HEIGHT {
            return (orig_width, orig_height);
        }
        
        // 计算缩放比例
        let scale_w = MAX_ENCODER_WIDTH as f32 / orig_width as f32;
        let scale_h = MAX_ENCODER_HEIGHT as f32 / orig_height as f32;
        let scale = scale_w.min(scale_h);
        
        let new_width = ((orig_width as f32 * scale) as usize) & !1;
        let new_height = ((orig_height as f32 * scale) as usize) & !1;
        
        println!("Resolution {}x{} scaled to {}x{} to fit encoder limits", 
            orig_width, orig_height, new_width, new_height);
        
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
    pub fn start_capture(&self, sender: mpsc::Sender<CapturedFrame>, target_fps: u32) {
        let target_width = self.width;
        let target_height = self.height;
        let frame_duration = Duration::from_millis(1000 / target_fps as u64);
        let needs_scaling = self.capture_width != self.width || self.capture_height != self.height;

        // 在独立线程中运行捕获
        std::thread::spawn(move || {
            use scrap::{Capturer, Display};
            
            // 创建捕获器
            let display = match Display::primary() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("No primary display found: {}", e);
                    return;
                }
            };
            
            let capture_width = display.width();
            let capture_height = display.height();
            
            let mut capturer = match Capturer::new(display) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to create capturer: {}", e);
                    return;
                }
            };

            let mut last_frame_time = Instant::now();

            loop {
                let now = Instant::now();
                let elapsed = now.duration_since(last_frame_time);

                if elapsed < frame_duration {
                    std::thread::sleep(frame_duration - elapsed);
                }

                // 捕获帧
                match capturer.frame() {
                    Ok(frame) => {
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

                        if sender.try_send(frame_data).is_err() {
                            // 通道已满或关闭，跳过此帧
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

/// H.264 视频编码器 (使用 openh264)
pub struct Vp8Encoder {
    encoder: openh264::encoder::Encoder,
    width: usize,
    height: usize,
}

impl Vp8Encoder {
    /// 创建新的 H.264 编码器
    /// 
    /// # Arguments
    /// * `width` - 视频宽度
    /// * `height` - 视频高度
    /// * `bitrate` - 目标比特率 (bps)，默认建议 2000000 (2 Mbps)
    pub fn new(width: usize, height: usize, target_bps: u32) -> anyhow::Result<Self> {
        use openh264::encoder::{Encoder, EncoderConfig, BitRate};
        use openh264::OpenH264API;
        
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(target_bps))
            .skip_frames(false);
        
        let api = OpenH264API::from_source();
        let encoder = Encoder::with_api_config(api, config)?;
        
        Ok(Self {
            encoder,
            width,
            height,
        })
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
        
        // 编码帧
        let bitstream = self.encoder.encode(&yuv_source)?;
        Ok(bitstream.to_vec())
    }
    
    /// 从 BGRA 帧直接编码
    /// 
    /// # Arguments
    /// * `frame` - BGRA 格式的捕获帧
    /// 
    /// # Returns
    /// 编码后的 H.264 数据
    pub fn encode_frame(&mut self, frame: &CapturedFrame) -> anyhow::Result<Vec<u8>> {
        // 1. BGRA -> I420
        let i420 = bgra_to_i420(&frame.data, frame.width, frame.height);
        
        // 2. I420 -> H.264
        self.encode_i420(&i420)
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
pub fn create_frame_pipeline(
    target_fps: u32, 
    bitrate: u32
) -> anyhow::Result<(ScreenCapture, mpsc::Receiver<Vec<u8>>)> {
    let (frame_sender, mut frame_receiver) = mpsc::channel::<CapturedFrame>(100);
    let (output_sender, output_receiver) = mpsc::channel::<Vec<u8>>(100);

    let capture = ScreenCapture::new()?;
    capture.start_capture(frame_sender, target_fps);

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
