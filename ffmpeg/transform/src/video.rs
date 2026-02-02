/*!
    Video frame transformation.
*/

use ffmpeg_next::{
    software::scaling::{context::Context as ScalerContext, flag::Flags as ScalerFlags},
    util::frame::video::Video as VideoFrameFFmpeg,
};

use ffmpeg_types::{Error, PixelFormat, Result, VideoFrame};

/**
    Scaling algorithm for video resizing.
*/
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScalingAlgorithm {
    /// Nearest neighbor - fastest, lowest quality.
    Nearest,
    /// Bilinear interpolation - fast, acceptable quality.
    #[default]
    Bilinear,
    /// Bicubic interpolation - moderate speed, good quality.
    Bicubic,
    /// Lanczos resampling - slowest, highest quality.
    Lanczos,
}

impl ScalingAlgorithm {
    fn to_ffmpeg_flags(self) -> ScalerFlags {
        match self {
            Self::Nearest => ScalerFlags::POINT,
            Self::Bilinear => ScalerFlags::BILINEAR,
            Self::Bicubic => ScalerFlags::BICUBIC,
            Self::Lanczos => ScalerFlags::LANCZOS,
        }
    }
}

/**
    Configuration for video transformation.
*/
#[derive(Clone, Debug)]
pub struct VideoTransformConfig {
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Target pixel format.
    pub format: PixelFormat,
    /// Scaling algorithm to use.
    pub algorithm: ScalingAlgorithm,
}

impl VideoTransformConfig {
    /**
        Create a new video transform configuration.
    */
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        Self {
            width,
            height,
            format,
            algorithm: ScalingAlgorithm::default(),
        }
    }

    /**
        Create configuration for BGRA output (common for display).
    */
    pub fn to_bgra(width: u32, height: u32) -> Self {
        Self::new(width, height, PixelFormat::Bgra)
    }

    /**
        Create configuration for RGBA output.
    */
    pub fn to_rgba(width: u32, height: u32) -> Self {
        Self::new(width, height, PixelFormat::Rgba)
    }

    /**
        Set the scaling algorithm.
    */
    pub fn with_algorithm(mut self, algorithm: ScalingAlgorithm) -> Self {
        self.algorithm = algorithm;
        self
    }
}

/**
    Video frame transformer.

    Converts video frames between formats, handling:
    - Pixel format conversion (YUV → RGB, etc.)
    - Scaling to different dimensions
    - Stride handling

    The scaler context is lazily initialized on first use and
    automatically reinitialized if the input format changes.
*/
pub struct VideoTransform {
    config: VideoTransformConfig,
    /// Cached scaler context and the input format it was created for.
    scaler_state: Option<ScalerState>,
}

struct ScalerState {
    context: ScalerContext,
    src_width: u32,
    src_height: u32,
    src_format: PixelFormat,
}

impl VideoTransform {
    /**
        Create a new video transformer with the given configuration.
    */
    pub fn new(config: VideoTransformConfig) -> Self {
        Self {
            config,
            scaler_state: None,
        }
    }

    /**
        Get the target configuration.
    */
    pub fn config(&self) -> &VideoTransformConfig {
        &self.config
    }

    /**
        Transform a video frame to the target format.

        The scaler is lazily initialized on first call and reused for
        subsequent frames with the same input format. If the input format
        changes, the scaler is automatically reinitialized.
    */
    pub fn transform(&mut self, frame: &VideoFrame) -> Result<VideoFrame> {
        // Validate input
        if frame.width == 0 || frame.height == 0 {
            return Err(Error::invalid_data("input frame has zero dimensions"));
        }

        if frame.data.is_empty() {
            return Err(Error::invalid_data("input frame has no data"));
        }

        // Check if we need to (re)initialize the scaler
        let needs_init = match &self.scaler_state {
            None => true,
            Some(state) => {
                state.src_width != frame.width
                    || state.src_height != frame.height
                    || state.src_format != frame.format
            }
        };

        if needs_init {
            self.init_scaler(frame.width, frame.height, frame.format)?;
        }

        // Perform the transformation
        self.scale_frame(frame)
    }

    /**
        Initialize or reinitialize the scaler for the given input format.
    */
    fn init_scaler(
        &mut self,
        src_width: u32,
        src_height: u32,
        src_format: PixelFormat,
    ) -> Result<()> {
        let src_pixel = pixel_format_to_ffmpeg(src_format)?;
        let dst_pixel = pixel_format_to_ffmpeg(self.config.format)?;

        let context = ScalerContext::get(
            src_pixel,
            src_width,
            src_height,
            dst_pixel,
            self.config.width,
            self.config.height,
            self.config.algorithm.to_ffmpeg_flags(),
        )
        .map_err(|e| Error::codec(format!("failed to create scaler: {}", e)))?;

        self.scaler_state = Some(ScalerState {
            context,
            src_width,
            src_height,
            src_format,
        });

        Ok(())
    }

    /**
        Scale a frame using the initialized scaler.
    */
    fn scale_frame(&mut self, frame: &VideoFrame) -> Result<VideoFrame> {
        let state = self.scaler_state.as_mut().expect("scaler not initialized");

        // Create FFmpeg source frame
        let src_pixel = pixel_format_to_ffmpeg(frame.format)?;
        let mut src_frame = VideoFrameFFmpeg::new(src_pixel, frame.width, frame.height);

        // Copy input data into FFmpeg frame
        copy_data_to_ffmpeg_frame(&mut src_frame, frame)?;

        // Create destination frame
        let dst_pixel = pixel_format_to_ffmpeg(self.config.format)?;
        let mut dst_frame = VideoFrameFFmpeg::new(dst_pixel, self.config.width, self.config.height);

        // Run the scaler
        state
            .context
            .run(&src_frame, &mut dst_frame)
            .map_err(|e| Error::codec(format!("scaling failed: {}", e)))?;

        // Copy output data from FFmpeg frame
        let data = copy_data_from_ffmpeg_frame(&dst_frame, self.config.format)?;

        Ok(VideoFrame::new(
            data,
            self.config.width,
            self.config.height,
            self.config.format,
            frame.pts,
            frame.time_base,
        ))
    }
}

/**
    Convert our PixelFormat to FFmpeg's Pixel format.
*/
fn pixel_format_to_ffmpeg(format: PixelFormat) -> Result<ffmpeg_next::format::Pixel> {
    use ffmpeg_next::format::Pixel;

    match format {
        PixelFormat::Yuv420p => Ok(Pixel::YUV420P),
        PixelFormat::Nv12 => Ok(Pixel::NV12),
        PixelFormat::Bgra => Ok(Pixel::BGRA),
        PixelFormat::Rgba => Ok(Pixel::RGBA),
        PixelFormat::Rgb24 => Ok(Pixel::RGB24),
        PixelFormat::Bgr24 => Ok(Pixel::BGR24),
        PixelFormat::Yuv422p => Ok(Pixel::YUV422P),
        PixelFormat::Yuv444p => Ok(Pixel::YUV444P),
        PixelFormat::Yuv420p10 => Ok(Pixel::YUV420P10LE),
        PixelFormat::P010le => Ok(Pixel::P010LE),
        _ => Err(Error::unsupported_format(format!(
            "pixel format {:?} not supported",
            format
        ))),
    }
}

/**
    Copy data from our VideoFrame into an FFmpeg frame.
*/
fn copy_data_to_ffmpeg_frame(dst: &mut VideoFrameFFmpeg, src: &VideoFrame) -> Result<()> {
    match src.format {
        // Packed formats - single plane
        PixelFormat::Bgra | PixelFormat::Rgba => {
            let bytes_per_pixel = 4;
            let dst_stride = dst.stride(0);
            let dst_data = dst.data_mut(0);

            for y in 0..src.height as usize {
                let src_row_start = y * src.width as usize * bytes_per_pixel;
                let src_row_end = src_row_start + src.width as usize * bytes_per_pixel;
                let dst_row_start = y * dst_stride;

                dst_data[dst_row_start..dst_row_start + src.width as usize * bytes_per_pixel]
                    .copy_from_slice(&src.data[src_row_start..src_row_end]);
            }
            Ok(())
        }

        PixelFormat::Rgb24 | PixelFormat::Bgr24 => {
            let bytes_per_pixel = 3;
            let dst_stride = dst.stride(0);
            let dst_data = dst.data_mut(0);

            for y in 0..src.height as usize {
                let src_row_start = y * src.width as usize * bytes_per_pixel;
                let src_row_end = src_row_start + src.width as usize * bytes_per_pixel;
                let dst_row_start = y * dst_stride;

                dst_data[dst_row_start..dst_row_start + src.width as usize * bytes_per_pixel]
                    .copy_from_slice(&src.data[src_row_start..src_row_end]);
            }
            Ok(())
        }

        // Planar YUV formats
        PixelFormat::Yuv420p
        | PixelFormat::Yuv422p
        | PixelFormat::Yuv444p
        | PixelFormat::Yuv420p10 => {
            let width = src.width as usize;
            let height = src.height as usize;

            let (uv_width, uv_height) = match src.format {
                PixelFormat::Yuv420p | PixelFormat::Yuv420p10 => (width / 2, height / 2),
                PixelFormat::Yuv422p => (width / 2, height),
                PixelFormat::Yuv444p => (width, height),
                _ => unreachable!(),
            };

            let bytes_per_sample = if src.format == PixelFormat::Yuv420p10 {
                2
            } else {
                1
            };

            // Calculate source offsets
            let y_size = width * height * bytes_per_sample;
            let uv_size = uv_width * uv_height * bytes_per_sample;

            // Copy Y plane
            let y_stride = dst.stride(0);
            let y_data = dst.data_mut(0);
            for y in 0..height {
                let src_start = y * width * bytes_per_sample;
                let dst_start = y * y_stride;
                y_data[dst_start..dst_start + width * bytes_per_sample]
                    .copy_from_slice(&src.data[src_start..src_start + width * bytes_per_sample]);
            }

            // Copy U plane
            let u_stride = dst.stride(1);
            let u_data = dst.data_mut(1);
            for y in 0..uv_height {
                let src_start = y_size + y * uv_width * bytes_per_sample;
                let dst_start = y * u_stride;
                u_data[dst_start..dst_start + uv_width * bytes_per_sample]
                    .copy_from_slice(&src.data[src_start..src_start + uv_width * bytes_per_sample]);
            }

            // Copy V plane
            let v_stride = dst.stride(2);
            let v_data = dst.data_mut(2);
            for y in 0..uv_height {
                let src_start = y_size + uv_size + y * uv_width * bytes_per_sample;
                let dst_start = y * v_stride;
                v_data[dst_start..dst_start + uv_width * bytes_per_sample]
                    .copy_from_slice(&src.data[src_start..src_start + uv_width * bytes_per_sample]);
            }

            Ok(())
        }

        // NV12 - semi-planar 8-bit
        PixelFormat::Nv12 => {
            let width = src.width as usize;
            let height = src.height as usize;
            let y_size = width * height;

            // Copy Y plane
            let y_stride = dst.stride(0);
            let y_data = dst.data_mut(0);
            for y in 0..height {
                let src_start = y * width;
                let dst_start = y * y_stride;
                y_data[dst_start..dst_start + width]
                    .copy_from_slice(&src.data[src_start..src_start + width]);
            }

            // Copy UV plane
            let uv_stride = dst.stride(1);
            let uv_data = dst.data_mut(1);
            let uv_height = height / 2;
            for y in 0..uv_height {
                let src_start = y_size + y * width;
                let dst_start = y * uv_stride;
                uv_data[dst_start..dst_start + width]
                    .copy_from_slice(&src.data[src_start..src_start + width]);
            }

            Ok(())
        }

        // P010 - semi-planar 10-bit (16-bit storage)
        PixelFormat::P010le => {
            let width = src.width as usize;
            let height = src.height as usize;
            let bytes_per_sample = 2;
            let y_size = width * height * bytes_per_sample;

            // Copy Y plane
            let y_stride = dst.stride(0);
            let y_data = dst.data_mut(0);
            for y in 0..height {
                let src_start = y * width * bytes_per_sample;
                let dst_start = y * y_stride;
                let row_bytes = width * bytes_per_sample;
                y_data[dst_start..dst_start + row_bytes]
                    .copy_from_slice(&src.data[src_start..src_start + row_bytes]);
            }

            // Copy UV plane
            let uv_stride = dst.stride(1);
            let uv_data = dst.data_mut(1);
            let uv_height = height / 2;
            for y in 0..uv_height {
                let src_start = y_size + y * width * bytes_per_sample;
                let dst_start = y * uv_stride;
                let row_bytes = width * bytes_per_sample;
                uv_data[dst_start..dst_start + row_bytes]
                    .copy_from_slice(&src.data[src_start..src_start + row_bytes]);
            }

            Ok(())
        }

        _ => Err(Error::unsupported_format(format!(
            "pixel format {:?} not supported for input",
            src.format
        ))),
    }
}

/**
    Copy data from an FFmpeg frame to a contiguous buffer.
*/
fn copy_data_from_ffmpeg_frame(frame: &VideoFrameFFmpeg, format: PixelFormat) -> Result<Vec<u8>> {
    match format {
        // Packed formats - single plane
        PixelFormat::Bgra | PixelFormat::Rgba => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let bytes_per_pixel = 4;
            let stride = frame.stride(0);
            let data = frame.data(0);

            let mut output = Vec::with_capacity(width * height * bytes_per_pixel);

            for y in 0..height {
                let row_start = y * stride;
                let row_end = row_start + width * bytes_per_pixel;
                output.extend_from_slice(&data[row_start..row_end]);
            }

            Ok(output)
        }

        PixelFormat::Rgb24 | PixelFormat::Bgr24 => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let bytes_per_pixel = 3;
            let stride = frame.stride(0);
            let data = frame.data(0);

            let mut output = Vec::with_capacity(width * height * bytes_per_pixel);

            for y in 0..height {
                let row_start = y * stride;
                let row_end = row_start + width * bytes_per_pixel;
                output.extend_from_slice(&data[row_start..row_end]);
            }

            Ok(output)
        }

        // Planar YUV formats
        PixelFormat::Yuv420p
        | PixelFormat::Yuv422p
        | PixelFormat::Yuv444p
        | PixelFormat::Yuv420p10 => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;

            let (uv_width, uv_height) = match format {
                PixelFormat::Yuv420p | PixelFormat::Yuv420p10 => (width / 2, height / 2),
                PixelFormat::Yuv422p => (width / 2, height),
                PixelFormat::Yuv444p => (width, height),
                _ => unreachable!(),
            };

            let bytes_per_sample = if format == PixelFormat::Yuv420p10 {
                2
            } else {
                1
            };
            let total_size = (width * height + 2 * uv_width * uv_height) * bytes_per_sample;

            let mut output = Vec::with_capacity(total_size);

            // Copy Y plane
            let y_stride = frame.stride(0);
            let y_data = frame.data(0);
            for y in 0..height {
                let row_start = y * y_stride;
                output.extend_from_slice(&y_data[row_start..row_start + width * bytes_per_sample]);
            }

            // Copy U plane
            let u_stride = frame.stride(1);
            let u_data = frame.data(1);
            for y in 0..uv_height {
                let row_start = y * u_stride;
                output
                    .extend_from_slice(&u_data[row_start..row_start + uv_width * bytes_per_sample]);
            }

            // Copy V plane
            let v_stride = frame.stride(2);
            let v_data = frame.data(2);
            for y in 0..uv_height {
                let row_start = y * v_stride;
                output
                    .extend_from_slice(&v_data[row_start..row_start + uv_width * bytes_per_sample]);
            }

            Ok(output)
        }

        // NV12 - semi-planar
        PixelFormat::Nv12 => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let y_size = width * height;
            let uv_size = width * (height / 2);

            let mut output = Vec::with_capacity(y_size + uv_size);

            // Copy Y plane
            let y_stride = frame.stride(0);
            let y_data = frame.data(0);
            for y in 0..height {
                let row_start = y * y_stride;
                output.extend_from_slice(&y_data[row_start..row_start + width]);
            }

            // Copy UV plane
            let uv_stride = frame.stride(1);
            let uv_data = frame.data(1);
            for y in 0..(height / 2) {
                let row_start = y * uv_stride;
                output.extend_from_slice(&uv_data[row_start..row_start + width]);
            }

            Ok(output)
        }

        _ => Err(Error::unsupported_format(format!(
            "pixel format {:?} not supported for output",
            format
        ))),
    }
}

impl std::fmt::Debug for VideoTransform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoTransform")
            .field("config", &self.config)
            .field("initialized", &self.scaler_state.is_some())
            .finish()
    }
}
