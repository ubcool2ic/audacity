use au_core::AudioBuffer;
use crate::IoError;
use libloading::{Library, Symbol};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;

pub const SUPPORTED: &str =
    "FFmpeg 7.x (libavformat 61, libavcodec 61, libavutil 59, libswresample 5)";

const AVERROR_EOF: c_int = -541_478_725;
const AVERROR_EAGAIN: c_int = -11;
const AV_SAMPLE_FMT_FLTP: c_int = 8;
const AVMEDIA_TYPE_AUDIO: c_int = 1;
const AVIO_FLAG_WRITE: c_int = 2;
const AVFMT_NOFILE: c_int = 1;
const AVFMT_GLOBALHEADER: c_int = 64;
const AV_CODEC_FLAG_GLOBAL_HEADER: c_int = 1 << 22;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct AVRational {
    num: i32,
    den: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AVChannelLayout {
    order: c_int,
    nb_channels: c_int,
    u: u64,
    opaque: *mut c_void,
}

type AvutilVersion = unsafe extern "C" fn() -> u32;
type AvVersionInfo = unsafe extern "C" fn() -> *const c_char;
type AvLogSetLevel = unsafe extern "C" fn(c_int);
type AvFrameAlloc = unsafe extern "C" fn() -> *mut c_void;
type AvFrameFree = unsafe extern "C" fn(*mut *mut c_void);
type AvFrameGetBuffer = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type AvFrameMakeWritable = unsafe extern "C" fn(*mut c_void) -> c_int;
type AvChannelLayoutDefault = unsafe extern "C" fn(*mut AVChannelLayout, c_int);
type AvChannelLayoutCopy =
    unsafe extern "C" fn(*mut AVChannelLayout, *const AVChannelLayout) -> c_int;
type AvChannelLayoutUninit = unsafe extern "C" fn(*mut AVChannelLayout);
type AvOptSetInt =
    unsafe extern "C" fn(*mut c_void, *const c_char, i64, c_int) -> c_int;
type AvStrerror = unsafe extern "C" fn(c_int, *mut c_char, usize) -> c_int;

type SwresampleVersion = unsafe extern "C" fn() -> u32;
type SwrAllocSetOpts2 = unsafe extern "C" fn(
    *mut *mut c_void,
    *const AVChannelLayout,
    c_int,
    c_int,
    *const AVChannelLayout,
    c_int,
    c_int,
    c_int,
    *mut c_void,
) -> c_int;
type SwrInit = unsafe extern "C" fn(*mut c_void) -> c_int;
type SwrConvert = unsafe extern "C" fn(
    *mut c_void,
    *const *mut u8,
    c_int,
    *const *const u8,
    c_int,
) -> c_int;
type SwrFree = unsafe extern "C" fn(*mut *mut c_void);

type AvcodecVersion = unsafe extern "C" fn() -> u32;
type AvcodecFindEncoderByName = unsafe extern "C" fn(*const c_char) -> *const c_void;
type AvcodecAllocContext3 = unsafe extern "C" fn(*const c_void) -> *mut c_void;
type AvcodecFreeContext = unsafe extern "C" fn(*mut *mut c_void);
type AvcodecParametersToContext =
    unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int;
type AvcodecParametersFromContext =
    unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int;
type AvcodecOpen2 =
    unsafe extern "C" fn(*mut c_void, *const c_void, *mut *mut c_void) -> c_int;
type AvcodecSendPacket = unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int;
type AvcodecReceiveFrame = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type AvcodecSendFrame = unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int;
type AvcodecReceivePacket = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type AvPacketAlloc = unsafe extern "C" fn() -> *mut c_void;
type AvPacketFree = unsafe extern "C" fn(*mut *mut c_void);
type AvPacketUnref = unsafe extern "C" fn(*mut c_void);
type AvPacketRescaleTs = unsafe extern "C" fn(*mut c_void, AVRational, AVRational);

type AvformatVersion = unsafe extern "C" fn() -> u32;
type AvformatOpenInput =
    unsafe extern "C" fn(*mut *mut c_void, *const c_char, *const c_void, *mut *mut c_void)
        -> c_int;
type AvformatFindStreamInfo =
    unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> c_int;
type AvFindBestStream = unsafe extern "C" fn(
    *mut c_void,
    c_int,
    c_int,
    c_int,
    *mut *const c_void,
    c_int,
) -> c_int;
type AvReadFrame = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type AvformatCloseInput = unsafe extern "C" fn(*mut *mut c_void);
type AvformatAllocOutputContext2 =
    unsafe extern "C" fn(*mut *mut c_void, *const c_void, *const c_char, *const c_char) -> c_int;
type AvformatNewStream = unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void;
type AvioOpen = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int) -> c_int;
type AvioClosep = unsafe extern "C" fn(*mut *mut c_void) -> c_int;
type AvformatWriteHeader = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> c_int;
type AvInterleavedWriteFrame = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type AvWriteTrailer = unsafe extern "C" fn(*mut c_void) -> c_int;
type AvformatFreeContext = unsafe extern "C" fn(*mut c_void);

#[allow(dead_code)] // version/log entry points are used only during loading
pub struct Ffmpeg {
    _avutil_library: Library,
    _swresample_library: Library,
    _avcodec_library: Library,
    _avformat_library: Library,

    avutil_version: AvutilVersion,
    av_version_info: AvVersionInfo,
    av_log_set_level: AvLogSetLevel,
    av_frame_alloc: AvFrameAlloc,
    av_frame_free: AvFrameFree,
    av_frame_get_buffer: AvFrameGetBuffer,
    av_frame_make_writable: AvFrameMakeWritable,
    av_channel_layout_default: AvChannelLayoutDefault,
    av_channel_layout_copy: AvChannelLayoutCopy,
    av_channel_layout_uninit: AvChannelLayoutUninit,
    av_opt_set_int: AvOptSetInt,
    av_strerror: AvStrerror,

    swresample_version: SwresampleVersion,
    swr_alloc_set_opts2: SwrAllocSetOpts2,
    swr_init: SwrInit,
    swr_convert: SwrConvert,
    swr_free: SwrFree,

    avcodec_version: AvcodecVersion,
    avcodec_find_encoder_by_name: AvcodecFindEncoderByName,
    avcodec_alloc_context3: AvcodecAllocContext3,
    avcodec_free_context: AvcodecFreeContext,
    avcodec_parameters_to_context: AvcodecParametersToContext,
    avcodec_parameters_from_context: AvcodecParametersFromContext,
    avcodec_open2: AvcodecOpen2,
    avcodec_send_packet: AvcodecSendPacket,
    avcodec_receive_frame: AvcodecReceiveFrame,
    avcodec_send_frame: AvcodecSendFrame,
    avcodec_receive_packet: AvcodecReceivePacket,
    av_packet_alloc: AvPacketAlloc,
    av_packet_free: AvPacketFree,
    av_packet_unref: AvPacketUnref,
    av_packet_rescale_ts: AvPacketRescaleTs,

    avformat_version: AvformatVersion,
    avformat_open_input: AvformatOpenInput,
    avformat_find_stream_info: AvformatFindStreamInfo,
    av_find_best_stream: AvFindBestStream,
    av_read_frame: AvReadFrame,
    avformat_close_input: AvformatCloseInput,
    avformat_alloc_output_context2: AvformatAllocOutputContext2,
    avformat_new_stream: AvformatNewStream,
    avio_open: AvioOpen,
    avio_closep: AvioClosep,
    avformat_write_header: AvformatWriteHeader,
    av_interleaved_write_frame: AvInterleavedWriteFrame,
    av_write_trailer: AvWriteTrailer,
    avformat_free_context: AvformatFreeContext,
}

// SAFETY: The libraries remain alive for the lifetime of this immutable function table.
// FFmpeg's APIs used here are designed for concurrent use with independent contexts.
unsafe impl Send for Ffmpeg {}
// SAFETY: See the Send implementation; each import/export call owns its FFmpeg contexts.
unsafe impl Sync for Ffmpeg {}

/// FFmpeg 7 library file names (avutil, swresample, avcodec, avformat) per platform.
#[cfg(windows)]
const LIB_NAMES: [&str; 4] = ["avutil-59.dll", "swresample-5.dll", "avcodec-61.dll", "avformat-61.dll"];
#[cfg(target_os = "macos")]
const LIB_NAMES: [&str; 4] = ["libavutil.59.dylib", "libswresample.5.dylib", "libavcodec.61.dylib", "libavformat.61.dylib"];
#[cfg(not(any(windows, target_os = "macos")))]
const LIB_NAMES: [&str; 4] = ["libavutil.so.59", "libswresample.so.5", "libavcodec.so.61", "libavformat.so.61"];

/// Extra directories tried after the system loader, like Audacity's FFmpeg lookup on Windows
/// (next to the executable, then "FFmpeg for Audacity" under Program Files).
fn fallback_dirs() -> Vec<std::path::PathBuf> {
    #[allow(unused_mut)]
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            dirs.push(d);
        }
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                dirs.push(std::path::PathBuf::from(&pf).join("FFmpeg for Audacity"));
                dirs.push(std::path::PathBuf::from(pf).join("FFmpeg").join("bin"));
            }
        }
    }
    dirs
}

fn load_library(dir: Option<&Path>, soname: &str) -> Result<Library, String> {
    let path = dir.map(|directory| directory.join(soname));
    let target = path
        .as_deref()
        .map(|path| path.as_os_str())
        .unwrap_or_else(|| std::ffi::OsStr::new(soname));

    // SAFETY: The returned library is stored in Ffmpeg and outlives every copied function pointer.
    unsafe { Library::new(target) }.map_err(|error| {
        match path {
            Some(path) => format!("{}: {error}", path.display()),
            None => format!("{soname} (system loader): {error}"),
        }
    })
}

fn load_symbol<T: Copy>(library: &Library, name: &'static [u8]) -> Result<T, String> {
    // SAFETY: The requested symbol names and function pointer types match the FFmpeg C ABI.
    let symbol: Symbol<'_, T> =
        unsafe { library.get(name) }.map_err(|error| format!("{}: {error}", String::from_utf8_lossy(name)))?;
    Ok(*symbol)
}

impl Ffmpeg {
    /// Loads from `dir` (the files libavformat.so.61 etc. inside it) or, when None, by soname via
    /// the system loader.
    pub fn load_from(dir: Option<&Path>) -> Result<Ffmpeg, String> {
        let [n_util, n_swr, n_codec, n_format] = LIB_NAMES;
        let avutil_library = load_library(dir, n_util)?;
        let swresample_library = load_library(dir, n_swr)?;
        let avcodec_library = load_library(dir, n_codec)?;
        let avformat_library = load_library(dir, n_format)?;

        macro_rules! symbol {
            ($library:expr, $name:literal, $ty:ty) => {
                load_symbol::<$ty>($library, concat!($name, "\0").as_bytes())?
            };
        }

        let avutil_version =
            symbol!(&avutil_library, "avutil_version", AvutilVersion);
        let av_version_info = symbol!(&avutil_library, "av_version_info", AvVersionInfo);
        let av_log_set_level = symbol!(&avutil_library, "av_log_set_level", AvLogSetLevel);
        let av_frame_alloc = symbol!(&avutil_library, "av_frame_alloc", AvFrameAlloc);
        let av_frame_free = symbol!(&avutil_library, "av_frame_free", AvFrameFree);
        let av_frame_get_buffer =
            symbol!(&avutil_library, "av_frame_get_buffer", AvFrameGetBuffer);
        let av_frame_make_writable =
            symbol!(&avutil_library, "av_frame_make_writable", AvFrameMakeWritable);
        let av_channel_layout_default = symbol!(
            &avutil_library,
            "av_channel_layout_default",
            AvChannelLayoutDefault
        );
        let av_channel_layout_copy = symbol!(
            &avutil_library,
            "av_channel_layout_copy",
            AvChannelLayoutCopy
        );
        let av_channel_layout_uninit = symbol!(
            &avutil_library,
            "av_channel_layout_uninit",
            AvChannelLayoutUninit
        );
        let av_opt_set_int = symbol!(&avutil_library, "av_opt_set_int", AvOptSetInt);
        let av_strerror = symbol!(&avutil_library, "av_strerror", AvStrerror);

        let swresample_version =
            symbol!(&swresample_library, "swresample_version", SwresampleVersion);
        let swr_alloc_set_opts2 =
            symbol!(&swresample_library, "swr_alloc_set_opts2", SwrAllocSetOpts2);
        let swr_init = symbol!(&swresample_library, "swr_init", SwrInit);
        let swr_convert = symbol!(&swresample_library, "swr_convert", SwrConvert);
        let swr_free = symbol!(&swresample_library, "swr_free", SwrFree);

        let avcodec_version =
            symbol!(&avcodec_library, "avcodec_version", AvcodecVersion);
        let avcodec_find_encoder_by_name = symbol!(
            &avcodec_library,
            "avcodec_find_encoder_by_name",
            AvcodecFindEncoderByName
        );
        let avcodec_alloc_context3 = symbol!(
            &avcodec_library,
            "avcodec_alloc_context3",
            AvcodecAllocContext3
        );
        let avcodec_free_context = symbol!(
            &avcodec_library,
            "avcodec_free_context",
            AvcodecFreeContext
        );
        let avcodec_parameters_to_context = symbol!(
            &avcodec_library,
            "avcodec_parameters_to_context",
            AvcodecParametersToContext
        );
        let avcodec_parameters_from_context = symbol!(
            &avcodec_library,
            "avcodec_parameters_from_context",
            AvcodecParametersFromContext
        );
        let avcodec_open2 = symbol!(&avcodec_library, "avcodec_open2", AvcodecOpen2);
        let avcodec_send_packet =
            symbol!(&avcodec_library, "avcodec_send_packet", AvcodecSendPacket);
        let avcodec_receive_frame =
            symbol!(&avcodec_library, "avcodec_receive_frame", AvcodecReceiveFrame);
        let avcodec_send_frame =
            symbol!(&avcodec_library, "avcodec_send_frame", AvcodecSendFrame);
        let avcodec_receive_packet =
            symbol!(&avcodec_library, "avcodec_receive_packet", AvcodecReceivePacket);
        let av_packet_alloc = symbol!(&avcodec_library, "av_packet_alloc", AvPacketAlloc);
        let av_packet_free = symbol!(&avcodec_library, "av_packet_free", AvPacketFree);
        let av_packet_unref = symbol!(&avcodec_library, "av_packet_unref", AvPacketUnref);
        let av_packet_rescale_ts =
            symbol!(&avcodec_library, "av_packet_rescale_ts", AvPacketRescaleTs);

        let avformat_version =
            symbol!(&avformat_library, "avformat_version", AvformatVersion);
        let avformat_open_input =
            symbol!(&avformat_library, "avformat_open_input", AvformatOpenInput);
        let avformat_find_stream_info = symbol!(
            &avformat_library,
            "avformat_find_stream_info",
            AvformatFindStreamInfo
        );
        let av_find_best_stream =
            symbol!(&avformat_library, "av_find_best_stream", AvFindBestStream);
        let av_read_frame = symbol!(&avformat_library, "av_read_frame", AvReadFrame);
        let avformat_close_input =
            symbol!(&avformat_library, "avformat_close_input", AvformatCloseInput);
        let avformat_alloc_output_context2 = symbol!(
            &avformat_library,
            "avformat_alloc_output_context2",
            AvformatAllocOutputContext2
        );
        let avformat_new_stream =
            symbol!(&avformat_library, "avformat_new_stream", AvformatNewStream);
        let avio_open = symbol!(&avformat_library, "avio_open", AvioOpen);
        let avio_closep = symbol!(&avformat_library, "avio_closep", AvioClosep);
        let avformat_write_header =
            symbol!(&avformat_library, "avformat_write_header", AvformatWriteHeader);
        let av_interleaved_write_frame = symbol!(
            &avformat_library,
            "av_interleaved_write_frame",
            AvInterleavedWriteFrame
        );
        let av_write_trailer =
            symbol!(&avformat_library, "av_write_trailer", AvWriteTrailer);
        let avformat_free_context =
            symbol!(&avformat_library, "avformat_free_context", AvformatFreeContext);

        // SAFETY: These version functions are resolved from the loaded libraries with their ABI types.
        let majors = unsafe {
            (
                avformat_version() >> 16,
                avcodec_version() >> 16,
                avutil_version() >> 16,
                swresample_version() >> 16,
            )
        };
        if majors != (61, 61, 59, 5) {
            return Err(format!(
                "unsupported FFmpeg version (libavformat {}, libavcodec {}, libavutil {}, libswresample {}); need {}",
                majors.0, majors.1, majors.2, majors.3, SUPPORTED
            ));
        }

        // SAFETY: The loaded libavutil function has the documented av_log_set_level ABI.
        unsafe { av_log_set_level(-8) };

        Ok(Ffmpeg {
            _avutil_library: avutil_library,
            _swresample_library: swresample_library,
            _avcodec_library: avcodec_library,
            _avformat_library: avformat_library,
            avutil_version,
            av_version_info,
            av_log_set_level,
            av_frame_alloc,
            av_frame_free,
            av_frame_get_buffer,
            av_frame_make_writable,
            av_channel_layout_default,
            av_channel_layout_copy,
            av_channel_layout_uninit,
            av_opt_set_int,
            av_strerror,
            swresample_version,
            swr_alloc_set_opts2,
            swr_init,
            swr_convert,
            swr_free,
            avcodec_version,
            avcodec_find_encoder_by_name,
            avcodec_alloc_context3,
            avcodec_free_context,
            avcodec_parameters_to_context,
            avcodec_parameters_from_context,
            avcodec_open2,
            avcodec_send_packet,
            avcodec_receive_frame,
            avcodec_send_frame,
            avcodec_receive_packet,
            av_packet_alloc,
            av_packet_free,
            av_packet_unref,
            av_packet_rescale_ts,
            avformat_version,
            avformat_open_input,
            avformat_find_stream_info,
            av_find_best_stream,
            av_read_frame,
            avformat_close_input,
            avformat_alloc_output_context2,
            avformat_new_stream,
            avio_open,
            avio_closep,
            avformat_write_header,
            av_interleaved_write_frame,
            av_write_trailer,
            avformat_free_context,
        })
    }

    pub fn version_info(&self) -> String {
        // SAFETY: The function pointer is from the live libavutil library.
        let version = unsafe { (self.av_version_info)() };
        if version.is_null() {
            return "unknown FFmpeg version".to_owned();
        }
        // SAFETY: av_version_info returns a process-lifetime NUL-terminated string.
        unsafe { CStr::from_ptr(version) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn import(&self, path: &Path) -> Result<AudioBuffer, IoError> {
        let path_c = path_cstring(path).map_err(IoError::Decode)?;
        let display_path = path.display().to_string();

        let mut format_ptr = null_mut();
        // SAFETY: format_ptr is a valid output slot and path_c is NUL-terminated.
        let result = unsafe {
            (self.avformat_open_input)(
                &mut format_ptr,
                path_c.as_ptr(),
                null(),
                null_mut(),
            )
        };
        let format = InputFormatGuard {
            ptr: format_ptr,
            close: self.avformat_close_input,
        };
        if result < 0 {
            return Err(self.decode_error(
                "could not open input",
                result,
            ));
        }
        if format.ptr.is_null() {
            return Err(IoError::Decode(format!(
                "could not open input {}",
                display_path
            )));
        }

        // SAFETY: format.ptr is a valid opened input context.
        let result =
            unsafe { (self.avformat_find_stream_info)(format.ptr, null_mut()) };
        if result < 0 {
            return Err(self.decode_error("could not read stream information", result));
        }

        let mut decoder = null();
        // SAFETY: format.ptr is valid and decoder is a writable output pointer.
        let stream_index = unsafe {
            (self.av_find_best_stream)(
                format.ptr,
                AVMEDIA_TYPE_AUDIO,
                -1,
                -1,
                &mut decoder,
                0,
            )
        };
        if stream_index < 0 {
            return Err(IoError::Decode(format!(
                "no audio stream in {display_path}"
            )));
        }
        if decoder.is_null() {
            return Err(IoError::Decode(format!(
                "no audio decoder available for {display_path}"
            )));
        }

        // SAFETY: The verified FFmpeg 7 AVFormatContext layout has these stream fields.
        let (streams, stream_count) = unsafe {
            (
                read_at::<*mut *mut c_void>(format.ptr, 48),
                read_at::<u32>(format.ptr, 44),
            )
        };
        if streams.is_null() || stream_index as u32 >= stream_count {
            return Err(IoError::Decode("invalid audio stream index".to_owned()));
        }
        // SAFETY: stream_index was returned by av_find_best_stream and is in bounds.
        let stream = unsafe { streams.add(stream_index as usize).read() };
        if stream.is_null() {
            return Err(IoError::Decode("invalid audio stream".to_owned()));
        }
        // SAFETY: AVStream.codecpar is at the verified FFmpeg 7 offset.
        let parameters = unsafe { read_at::<*mut c_void>(stream, 16) };

        // SAFETY: decoder is a valid decoder returned by av_find_best_stream.
        let codec_ptr = unsafe { (self.avcodec_alloc_context3)(decoder) };
        if codec_ptr.is_null() {
            return Err(IoError::Decode("could not allocate decoder context".to_owned()));
        }
        let codec = CodecContextGuard {
            ptr: codec_ptr,
            free: self.avcodec_free_context,
        };

        // SAFETY: codec and parameters are valid contexts for this stream.
        let result =
            unsafe { (self.avcodec_parameters_to_context)(codec.ptr, parameters) };
        if result < 0 {
            return Err(self.decode_error("could not configure decoder", result));
        }
        // SAFETY: codec and decoder are valid; options are not required here.
        let result = unsafe { (self.avcodec_open2)(codec.ptr, decoder, null_mut()) };
        if result < 0 {
            return Err(self.decode_error("could not open decoder", result));
        }

        // SAFETY: The functions are from the same live FFmpeg libraries.
        let frame_ptr = unsafe { (self.av_frame_alloc)() };
        if frame_ptr.is_null() {
            return Err(IoError::Decode("could not allocate audio frame".to_owned()));
        }
        let frame = FrameGuard {
            ptr: frame_ptr,
            free: self.av_frame_free,
            uninit_layout: self.av_channel_layout_uninit,
        };
        // SAFETY: The packet allocator is from the live libavcodec library.
        let packet_ptr = unsafe { (self.av_packet_alloc)() };
        if packet_ptr.is_null() {
            return Err(IoError::Decode("could not allocate audio packet".to_owned()));
        }
        let packet = PacketGuard {
            ptr: packet_ptr,
            free: self.av_packet_free,
        };

        let mut decoded = DecodedAudio {
            sample_rate: None,
            channels: Vec::new(),
            swr: None,
        };

        loop {
            // SAFETY: format and packet are valid for av_read_frame.
            let result = unsafe { (self.av_read_frame)(format.ptr, packet.ptr) };
            if result == AVERROR_EOF {
                break;
            }
            if result < 0 {
                return Err(self.decode_error("error reading input packet", result));
            }

            // SAFETY: AVPacket.stream_index is at the verified FFmpeg 7 offset.
            let packet_stream = unsafe { read_at::<c_int>(packet.ptr, 36) };
            if packet_stream != stream_index {
                // SAFETY: packet is a valid packet allocated by av_packet_alloc.
                unsafe { (self.av_packet_unref)(packet.ptr) };
                continue;
            }

            // SAFETY: codec and packet are valid decoder inputs.
            let mut result =
                unsafe { (self.avcodec_send_packet)(codec.ptr, packet.ptr) };
            if result == AVERROR_EAGAIN {
                self.receive_decoded(codec.ptr, frame.ptr, &mut decoded)?;
                // SAFETY: codec and packet remain valid after draining available frames.
                result = unsafe { (self.avcodec_send_packet)(codec.ptr, packet.ptr) };
            }
            // SAFETY: packet is valid and may be unreferenced after submission.
            unsafe { (self.av_packet_unref)(packet.ptr) };
            if result < 0 {
                return Err(self.decode_error("could not submit audio packet", result));
            }
            self.receive_decoded(codec.ptr, frame.ptr, &mut decoded)?;
        }

        // SAFETY: A null packet signals the decoder to drain delayed frames.
        let mut result =
            unsafe { (self.avcodec_send_packet)(codec.ptr, null()) };
        if result == AVERROR_EAGAIN {
            self.receive_decoded(codec.ptr, frame.ptr, &mut decoded)?;
            // SAFETY: The codec is valid and has been drained before retrying.
            result = unsafe { (self.avcodec_send_packet)(codec.ptr, null()) };
        }
        if result < 0 && result != AVERROR_EOF {
            return Err(self.decode_error("could not flush decoder", result));
        }
        self.receive_decoded(codec.ptr, frame.ptr, &mut decoded)?;
        self.flush_resampler(&mut decoded)?;

        if decoded.channels.is_empty()
            || decoded.channels.first().is_none_or(|channel| channel.is_empty())
        {
            return Err(IoError::Decode(format!(
                "no audio samples decoded from {display_path}"
            )));
        }

        Ok(AudioBuffer::from_channels(
            decoded.sample_rate.unwrap_or(0),
            decoded.channels,
        ))
    }

    pub fn export(
        &self,
        path: &Path,
        buffer: &AudioBuffer,
        format: ExportFormat,
        bit_rate: i64,
    ) -> Result<(), IoError> {
        if buffer.len() == 0 {
            return Err(IoError::Encode("cannot export an empty audio buffer".to_owned()));
        }
        if buffer.channels.is_empty() {
            return Err(IoError::Encode("audio buffer has no channels".to_owned()));
        }
        if format == ExportFormat::Ac3
            && !matches!(buffer.sample_rate, 48_000 | 44_100 | 32_000)
        {
            return Err(IoError::Encode(
                "AC3 export requires a sample rate of 48000, 44100, or 32000 Hz".to_owned(),
            ));
        }

        let rate = c_int::try_from(buffer.sample_rate)
            .ok()
            .filter(|rate| *rate > 0)
            .ok_or_else(|| IoError::Encode("invalid sample rate".to_owned()))?;
        let channel_count = c_int::try_from(buffer.n_channels())
            .ok()
            .filter(|channels| *channels > 0)
            .ok_or_else(|| IoError::Encode("invalid channel count".to_owned()))?;
        let path_c = path_cstring(path).map_err(IoError::Encode)?;
        let format_name = CString::new(format.container_name()).expect("static format name");
        let encoder_name = CString::new(format.encoder_name()).expect("static encoder name");

        let mut format_ptr = null_mut();
        // SAFETY: The format name and path are valid C strings and format_ptr is an output slot.
        let result = unsafe {
            (self.avformat_alloc_output_context2)(
                &mut format_ptr,
                null(),
                format_name.as_ptr(),
                path_c.as_ptr(),
            )
        };
        let output = OutputFormatGuard {
            ptr: format_ptr,
            close_io: self.avio_closep,
            free: self.avformat_free_context,
        };
        if result < 0 {
            return Err(self.encode_error("could not allocate output context", result));
        }
        if output.ptr.is_null() {
            return Err(IoError::Encode(
                "could not allocate output context".to_owned(),
            ));
        }

        // SAFETY: The encoder name is a valid NUL-terminated string.
        let encoder = unsafe { (self.avcodec_find_encoder_by_name)(encoder_name.as_ptr()) };
        if encoder.is_null() {
            return Err(IoError::Encode(format!(
                "FFmpeg encoder '{}' is unavailable",
                format.encoder_name()
            )));
        }

        // SAFETY: output.ptr and encoder are valid FFmpeg objects.
        let stream = unsafe { (self.avformat_new_stream)(output.ptr, null()) };
        if stream.is_null() {
            return Err(IoError::Encode("could not create output stream".to_owned()));
        }

        // SAFETY: encoder is a valid encoder descriptor.
        let codec_ptr = unsafe { (self.avcodec_alloc_context3)(encoder) };
        if codec_ptr.is_null() {
            return Err(IoError::Encode(
                "could not allocate encoder context".to_owned(),
            ));
        }
        let codec = CodecContextGuard {
            ptr: codec_ptr,
            free: self.avcodec_free_context,
        };

        // SAFETY: These are verified FFmpeg 7 AVCodecContext field offsets.
        unsafe {
            write_at(codec.ptr, 84, AVRational { num: 1, den: rate });
            write_at(codec.ptr, 344, rate);
            write_at(codec.ptr, 348, AV_SAMPLE_FMT_FLTP);
        }
        // SAFETY: The embedded channel layout is writable and channel_count is positive.
        let codec_layout =
            unsafe { field_ptr::<AVChannelLayout>(codec.ptr, 352) };
        // SAFETY: codec_layout points to the initialized embedded AVChannelLayout.
        unsafe { (self.av_channel_layout_default)(codec_layout, channel_count) };

        // SAFETY: av_opt_set_int accepts this valid codec context and option name.
        let result = unsafe {
            (self.av_opt_set_int)(codec.ptr, b"b\0".as_ptr().cast(), bit_rate, 0)
        };
        if result < 0 {
            return Err(self.encode_error("could not set encoder bit rate", result));
        }

        // SAFETY: AVFormatContext.oformat is at the verified FFmpeg 7 offset.
        let output_format = unsafe { read_at::<*const c_void>(output.ptr, 16) };
        if output_format.is_null() {
            return Err(IoError::Encode("output format is unavailable".to_owned()));
        }
        // SAFETY: AVOutputFormat.flags is at the verified FFmpeg 7 offset.
        let output_flags = unsafe { read_at::<c_int>(output_format.cast_mut(), 44) };
        if output_flags & AVFMT_GLOBALHEADER != 0 {
            // SAFETY: AVCodecContext.flags is at the verified FFmpeg 7 offset.
            let flags = unsafe { read_at::<c_int>(codec.ptr, 64) };
            // SAFETY: AVCodecContext.flags is writable at the verified FFmpeg 7 offset.
            unsafe {
                write_at(
                    codec.ptr,
                    64,
                    flags | AV_CODEC_FLAG_GLOBAL_HEADER,
                )
            };
        }

        // SAFETY: codec and encoder are valid; options are not required here.
        let result = unsafe { (self.avcodec_open2)(codec.ptr, encoder, null_mut()) };
        if result < 0 {
            return Err(self.encode_error("could not open encoder", result));
        }

        // SAFETY: stream.codecpar is at the verified FFmpeg 7 AVStream offset.
        let parameters = unsafe { read_at::<*mut c_void>(stream, 16) };
        // SAFETY: parameters and codec are valid output objects.
        let result =
            unsafe { (self.avcodec_parameters_from_context)(parameters, codec.ptr) };
        if result < 0 {
            return Err(self.encode_error("could not configure output stream", result));
        }
        // SAFETY: AVStream.time_base is writable at the verified FFmpeg 7 offset.
        unsafe { write_at(stream, 32, AVRational { num: 1, den: rate }) };

        if output_flags & AVFMT_NOFILE == 0 {
            // SAFETY: AVFormatContext.pb is a writable field at the verified FFmpeg 7 offset.
            let pb = unsafe { field_ptr::<*mut c_void>(output.ptr, 32) };
            // SAFETY: path_c is a valid output path and pb is a valid output slot.
            let result = unsafe { (self.avio_open)(pb, path_c.as_ptr(), AVIO_FLAG_WRITE) };
            if result < 0 {
                return Err(self.encode_error("could not open output file", result));
            }
        }

        // SAFETY: output.ptr is valid and no muxer options are required.
        let result = unsafe { (self.avformat_write_header)(output.ptr, null_mut()) };
        if result < 0 {
            return Err(self.encode_error("could not write output header", result));
        }
        // SAFETY: AVStream.time_base may be changed by the muxer during header writing.
        let stream_time_base = unsafe { read_at::<AVRational>(stream, 32) };
        let codec_time_base = AVRational { num: 1, den: rate };

        // SAFETY: Packet allocation uses the live libavcodec allocator.
        let packet_ptr = unsafe { (self.av_packet_alloc)() };
        if packet_ptr.is_null() {
            return Err(IoError::Encode("could not allocate output packet".to_owned()));
        }
        let packet = PacketGuard {
            ptr: packet_ptr,
            free: self.av_packet_free,
        };

        // SAFETY: AVCodecContext.frame_size is at the verified FFmpeg 7 offset.
        let configured_frame_size = unsafe { read_at::<c_int>(codec.ptr, 376) };
        let frame_size = if configured_frame_size > 0 {
            configured_frame_size as usize
        } else {
            1024
        };

        let total_frames = buffer.len();
        for start in (0..total_frames).step_by(frame_size) {
            let count = (total_frames - start).min(frame_size);
            // SAFETY: The frame allocator is from the live libavutil library.
            let frame_ptr = unsafe { (self.av_frame_alloc)() };
            if frame_ptr.is_null() {
                return Err(IoError::Encode("could not allocate audio frame".to_owned()));
            }
            let frame = FrameGuard {
                ptr: frame_ptr,
                free: self.av_frame_free,
                uninit_layout: self.av_channel_layout_uninit,
            };

            // SAFETY: These are verified FFmpeg 7 AVFrame field offsets.
            unsafe {
                write_at(frame.ptr, 112, frame_size as c_int);
                write_at(frame.ptr, 116, AV_SAMPLE_FMT_FLTP);
                write_at(frame.ptr, 136, start as i64);
                write_at(frame.ptr, 192, rate);
            }
            // SAFETY: The embedded frame channel layout is writable.
            let frame_layout =
                unsafe { field_ptr::<AVChannelLayout>(frame.ptr, 408) };
            // SAFETY: Both layouts are valid and the destination is an empty frame layout.
            let result = unsafe { (self.av_channel_layout_copy)(frame_layout, codec_layout) };
            if result < 0 {
                return Err(self.encode_error("could not copy frame channel layout", result));
            }
            // SAFETY: frame is configured with a valid sample format, count, and layout.
            let result = unsafe { (self.av_frame_get_buffer)(frame.ptr, 0) };
            if result < 0 {
                return Err(self.encode_error("could not allocate frame samples", result));
            }
            // SAFETY: frame has allocated sample buffers and may be written by the caller.
            let result = unsafe { (self.av_frame_make_writable)(frame.ptr) };
            if result < 0 {
                return Err(self.encode_error("could not make frame writable", result));
            }
            // SAFETY: AVFrame.extended_data is at the verified FFmpeg 7 offset.
            let extended_data = unsafe { read_at::<*mut *mut u8>(frame.ptr, 96) };
            if extended_data.is_null() {
                return Err(IoError::Encode("encoder frame has no sample planes".to_owned()));
            }

            for channel_index in 0..buffer.n_channels() {
                // SAFETY: The frame was allocated with channel_count planar sample planes.
                let plane = unsafe { extended_data.add(channel_index).read() };
                if plane.is_null() {
                    return Err(IoError::Encode("encoder frame has a null sample plane".to_owned()));
                }
                // SAFETY: Each plane has room for frame_size floats; final samples are padded.
                let samples =
                    unsafe { std::slice::from_raw_parts_mut(plane.cast::<f32>(), frame_size) };
                samples.fill(0.0);
                if let Some(source) = buffer.channels.get(channel_index) {
                    let available = source.len().saturating_sub(start).min(count);
                    samples[..available]
                        .copy_from_slice(&source[start..start + available]);
                }
            }

            self.send_encoded_frame(
                codec.ptr,
                frame.ptr,
                output.ptr,
                packet.ptr,
                codec_time_base,
                stream_time_base,
            )?;
        }

        self.send_encoded_frame(
            codec.ptr,
            null(),
            output.ptr,
            packet.ptr,
            codec_time_base,
            stream_time_base,
        )?;
        self.write_encoded_packets(
            codec.ptr,
            output.ptr,
            packet.ptr,
            codec_time_base,
            stream_time_base,
        )?;

        // SAFETY: output.ptr is valid and its header has been written.
        let result = unsafe { (self.av_write_trailer)(output.ptr) };
        if result < 0 {
            return Err(self.encode_error("could not write output trailer", result));
        }

        // SAFETY: AVFormatContext.pb is a valid output slot and avio_closep clears it.
        let pb = unsafe { field_ptr::<*mut c_void>(output.ptr, 32) };
        // SAFETY: The output AVIOContext was opened by avio_open when present.
        let result = unsafe { (self.avio_closep)(pb) };
        if result < 0 {
            return Err(self.encode_error("could not close output file", result));
        }

        Ok(())
    }

    fn receive_decoded(
        &self,
        codec: *mut c_void,
        frame: *mut c_void,
        decoded: &mut DecodedAudio,
    ) -> Result<(), IoError> {
        loop {
            // SAFETY: codec and frame are valid decoder objects.
            let result = unsafe { (self.avcodec_receive_frame)(codec, frame) };
            if result == AVERROR_EAGAIN || result == AVERROR_EOF {
                return Ok(());
            }
            if result < 0 {
                return Err(self.decode_error("could not receive decoded frame", result));
            }

            // SAFETY: These fields are at the verified FFmpeg 7 AVFrame offsets.
            let (sample_count, sample_format, sample_rate, layout_channels) = unsafe {
                (
                    read_at::<c_int>(frame, 112),
                    read_at::<c_int>(frame, 116),
                    read_at::<c_int>(frame, 192),
                    read_at::<c_int>(frame, 412),
                )
            };
            if sample_count <= 0 || sample_rate <= 0 || layout_channels <= 0 {
                return Err(IoError::Decode(
                    "decoder returned an invalid audio frame".to_owned(),
                ));
            }

            if decoded.swr.is_none() {
                let layout = unsafe { field_ptr::<AVChannelLayout>(frame, 408) };
                let mut swr_ptr = null_mut();
                // SAFETY: The frame layout is valid for both input and output; the converter
                // produces planar float samples at the frame's native rate and channel layout.
                let result = unsafe {
                    (self.swr_alloc_set_opts2)(
                        &mut swr_ptr,
                        layout,
                        AV_SAMPLE_FMT_FLTP,
                        sample_rate,
                        layout,
                        sample_format,
                        sample_rate,
                        0,
                        null_mut(),
                    )
                };
                let swr = SwrContextGuard {
                    ptr: swr_ptr,
                    free: self.swr_free,
                };
                if result < 0 {
                    return Err(self.decode_error("could not create audio converter", result));
                }
                if swr.ptr.is_null() {
                    return Err(IoError::Decode(
                        "could not create audio converter".to_owned(),
                    ));
                }
                // SAFETY: swr.ptr is a valid allocated resampling context.
                let result = unsafe { (self.swr_init)(swr.ptr) };
                if result < 0 {
                    return Err(self.decode_error("could not initialize audio converter", result));
                }
                decoded.sample_rate = Some(sample_rate as u32);
                decoded.channels = (0..layout_channels).map(|_| Vec::new()).collect();
                decoded.swr = Some(swr);
            } else if decoded.sample_rate != Some(sample_rate as u32)
                || decoded.channels.len() != layout_channels as usize
            {
                return Err(IoError::Decode(
                    "audio stream changes sample rate or channel layout".to_owned(),
                ));
            }

            let capacity = sample_count
                .checked_add(256)
                .ok_or_else(|| IoError::Decode("audio frame is too large".to_owned()))?;
            let mut scratch: Vec<Vec<f32>> = (0..layout_channels)
                .map(|_| vec![0.0; capacity as usize])
                .collect();
            let mut output_planes: Vec<*mut u8> = scratch
                .iter_mut()
                .map(|plane| plane.as_mut_ptr().cast::<u8>())
                .collect();
            // SAFETY: AVFrame.extended_data points to the input sample planes.
            let input_planes = unsafe { read_at::<*mut *mut u8>(frame, 96) };
            if input_planes.is_null() {
                return Err(IoError::Decode("decoded frame has no sample planes".to_owned()));
            }
            let swr_ptr = decoded.swr.as_ref().expect("converter was initialized").ptr;
            // SAFETY: All plane arrays remain alive and sized for this conversion call.
            let converted = unsafe {
                (self.swr_convert)(
                    swr_ptr,
                    output_planes.as_mut_ptr(),
                    capacity,
                    input_planes.cast::<*const u8>(),
                    sample_count,
                )
            };
            if converted < 0 {
                return Err(self.decode_error("could not convert decoded audio", converted));
            }
            for (output, scratch_plane) in decoded.channels.iter_mut().zip(scratch.iter()) {
                output.extend_from_slice(&scratch_plane[..converted as usize]);
            }
        }
    }

    fn flush_resampler(&self, decoded: &mut DecodedAudio) -> Result<(), IoError> {
        let Some(swr) = decoded.swr.as_ref() else {
            return Ok(());
        };
        let swr_ptr = swr.ptr;
        let channel_count = decoded.channels.len();
        loop {
            let capacity = 4096;
            let mut scratch: Vec<Vec<f32>> =
                (0..channel_count).map(|_| vec![0.0; capacity]).collect();
            let mut output_planes: Vec<*mut u8> = scratch
                .iter_mut()
                .map(|plane| plane.as_mut_ptr().cast::<u8>())
                .collect();
            // SAFETY: The output planes are valid and null input with zero samples flushes swr.
            let converted = unsafe {
                (self.swr_convert)(
                    swr_ptr,
                    output_planes.as_mut_ptr(),
                    capacity as c_int,
                    null(),
                    0,
                )
            };
            if converted < 0 {
                return Err(self.decode_error("could not flush audio converter", converted));
            }
            if converted == 0 {
                return Ok(());
            }
            for (output, scratch_plane) in decoded.channels.iter_mut().zip(scratch.iter()) {
                output.extend_from_slice(&scratch_plane[..converted as usize]);
            }
        }
    }

    fn send_encoded_frame(
        &self,
        codec: *mut c_void,
        frame: *const c_void,
        format: *mut c_void,
        packet: *mut c_void,
        codec_time_base: AVRational,
        stream_time_base: AVRational,
    ) -> Result<(), IoError> {
        // SAFETY: codec and frame are valid; a null frame signals encoder flushing.
        let mut result = unsafe { (self.avcodec_send_frame)(codec, frame) };
        if result == AVERROR_EAGAIN {
            self.write_encoded_packets(
                codec,
                format,
                packet,
                codec_time_base,
                stream_time_base,
            )?;
            // SAFETY: Available packets were drained before retrying the valid frame.
            result = unsafe { (self.avcodec_send_frame)(codec, frame) };
        }
        if result < 0 && result != AVERROR_EOF {
            return Err(self.encode_error("could not submit audio frame", result));
        }
        self.write_encoded_packets(
            codec,
            format,
            packet,
            codec_time_base,
            stream_time_base,
        )
    }

    fn write_encoded_packets(
        &self,
        codec: *mut c_void,
        format: *mut c_void,
        packet: *mut c_void,
        codec_time_base: AVRational,
        stream_time_base: AVRational,
    ) -> Result<(), IoError> {
        loop {
            // SAFETY: codec and packet are valid encoder objects.
            let result = unsafe { (self.avcodec_receive_packet)(codec, packet) };
            if result == AVERROR_EAGAIN || result == AVERROR_EOF {
                return Ok(());
            }
            if result < 0 {
                return Err(self.encode_error("could not receive encoded packet", result));
            }

            // SAFETY: AVPacket.stream_index is at the verified FFmpeg 7 offset.
            unsafe { write_at(packet, 36, 0 as c_int) };
            // SAFETY: packet timestamps and both time bases are valid.
            unsafe {
                (self.av_packet_rescale_ts)(packet, codec_time_base, stream_time_base)
            };
            // SAFETY: format and packet are valid objects; the muxer consumes the packet.
            let result = unsafe { (self.av_interleaved_write_frame)(format, packet) };
            // SAFETY: packet is valid; unref is safe after muxer submission.
            unsafe { (self.av_packet_unref)(packet) };
            if result < 0 {
                return Err(self.encode_error("could not write encoded packet", result));
            }
        }
    }

    fn decode_error(&self, context: &str, code: c_int) -> IoError {
        IoError::Decode(format!("{context}: {}", self.error_string(code)))
    }

    fn encode_error(&self, context: &str, code: c_int) -> IoError {
        IoError::Encode(format!("{context}: {}", self.error_string(code)))
    }

    fn error_string(&self, code: c_int) -> String {
        let mut buffer = [0 as c_char; 256];
        // SAFETY: buffer is writable and av_strerror receives its exact capacity.
        let result =
            unsafe { (self.av_strerror)(code, buffer.as_mut_ptr(), buffer.len()) };
        if result < 0 {
            return format!("FFmpeg error {code}");
        }
        // SAFETY: av_strerror writes a NUL-terminated message on success.
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }
}

fn path_cstring(path: &Path) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| format!("path contains a NUL byte: {}", path.display()))
}

unsafe fn read_at<T: Copy>(base: *mut c_void, offset: usize) -> T {
    // SAFETY: Callers provide a non-null FFmpeg object and an offset verified for its major version.
    unsafe { base.cast::<u8>().add(offset).cast::<T>().read() }
}

unsafe fn write_at<T>(base: *mut c_void, offset: usize, value: T) {
    // SAFETY: Callers provide a writable FFmpeg object and a verified, aligned field offset.
    unsafe { base.cast::<u8>().add(offset).cast::<T>().write(value) }
}

unsafe fn field_ptr<T>(base: *mut c_void, offset: usize) -> *mut T {
    // SAFETY: Callers provide a valid FFmpeg object and a verified field offset.
    unsafe { base.cast::<u8>().add(offset).cast::<T>() }
}

struct InputFormatGuard {
    ptr: *mut c_void,
    close: AvformatCloseInput,
}

impl Drop for InputFormatGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr is an opened input context owned by this guard.
            unsafe { (self.close)(&mut self.ptr) };
        }
    }
}

struct OutputFormatGuard {
    ptr: *mut c_void,
    close_io: AvioClosep,
    free: AvformatFreeContext,
}

impl Drop for OutputFormatGuard {
    fn drop(&mut self) {
        if self.ptr.is_null() {
            return;
        }
        // SAFETY: AVFormatContext.pb is at the verified FFmpeg 7 offset.
        let pb = unsafe { field_ptr::<*mut c_void>(self.ptr, 32) };
        // SAFETY: pb points to the context's writable AVIOContext slot.
        let has_io = unsafe { read_at::<*mut c_void>(self.ptr, 32) };
        if !has_io.is_null() {
            // SAFETY: This guard owns the AVIOContext opened for its output context.
            unsafe { (self.close_io)(pb) };
        }
        // SAFETY: ptr is an output context allocated by avformat_alloc_output_context2.
        unsafe { (self.free)(self.ptr) };
        self.ptr = null_mut();
    }
}

struct CodecContextGuard {
    ptr: *mut c_void,
    free: AvcodecFreeContext,
}

impl Drop for CodecContextGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr is an allocated codec context owned by this guard.
            unsafe { (self.free)(&mut self.ptr) };
        }
    }
}

struct FrameGuard {
    ptr: *mut c_void,
    free: AvFrameFree,
    uninit_layout: AvChannelLayoutUninit,
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: AVFrame.ch_layout is at the verified FFmpeg 7 offset and is safe to uninitialize.
            let layout = unsafe { field_ptr::<AVChannelLayout>(self.ptr, 408) };
            // SAFETY: layout is the frame's owned channel layout; frame_free tolerates an uninitialized layout.
            unsafe { (self.uninit_layout)(layout) };
            // SAFETY: ptr is an allocated AVFrame owned by this guard.
            unsafe { (self.free)(&mut self.ptr) };
        }
    }
}

struct PacketGuard {
    ptr: *mut c_void,
    free: AvPacketFree,
}

impl Drop for PacketGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr is an allocated AVPacket owned by this guard.
            unsafe { (self.free)(&mut self.ptr) };
        }
    }
}

struct SwrContextGuard {
    ptr: *mut c_void,
    free: SwrFree,
}

impl Drop for SwrContextGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr is an allocated SwrContext owned by this guard.
            unsafe { (self.free)(&mut self.ptr) };
        }
    }
}

struct DecodedAudio {
    sample_rate: Option<u32>,
    channels: Vec<Vec<f32>>,
    swr: Option<SwrContextGuard>,
}

pub fn get() -> Result<&'static Ffmpeg, IoError> {
    static INSTANCE: OnceLock<Result<Ffmpeg, String>> = OnceLock::new();
    match INSTANCE.get_or_init(|| match std::env::var_os("AU_FFMPEG_LIBDIR") {
        Some(directory) => Ffmpeg::load_from(Some(Path::new(&directory))),
        None => Ffmpeg::load_from(None).or_else(|first| {
            fallback_dirs()
                .iter()
                .find_map(|d| Ffmpeg::load_from(Some(d)).ok())
                .ok_or(first)
        }),
    }) {
        Ok(ffmpeg) => Ok(ffmpeg),
        Err(detail) => Err(IoError::FfmpegMissing(missing_message(detail))),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FfmpegStatus {
    Available { version: String },
    Missing { message: String },
}

pub fn status() -> FfmpegStatus {
    match get() {
        Ok(ffmpeg) => FfmpegStatus::Available {
            version: ffmpeg.version_info(),
        },
        Err(IoError::FfmpegMissing(message)) => FfmpegStatus::Missing { message },
        Err(error) => FfmpegStatus::Missing {
            message: error.to_string(),
        },
    }
}

pub fn missing_message(detail: &str) -> String {
    format!(
        "FFmpeg support is unavailable. Looked for {} in AU_FFMPEG_LIBDIR (when configured) \
         or the system library search path.\n\
         Loader error: {detail}\n\
         FFmpeg 7 is needed for M4A/AAC, WMA, AC3, and audio from video files.\n\
         On Debian/Ubuntu: sudo apt install ffmpeg (or libavformat61 libavcodec61 libswresample5).\n\
         WAV, FLAC, MP3 and Ogg import keep working without FFmpeg.",
        LIB_NAMES.join(", ")
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    M4a,
    Ac3,
    Wma,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::M4a => "m4a",
            Self::Ac3 => "ac3",
            Self::Wma => "wma",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::M4a => "M4A (AAC)",
            Self::Ac3 => "AC3",
            Self::Wma => "WMA (version 2)",
        }
    }

    pub fn all() -> [ExportFormat; 3] {
        [Self::M4a, Self::Ac3, Self::Wma]
    }

    pub fn default_bit_rate(self) -> i64 {
        match self {
            Self::M4a | Self::Ac3 => 192_000,
            Self::Wma => 128_000,
        }
    }

    fn container_name(self) -> &'static str {
        match self {
            Self::M4a => "ipod",
            Self::Ac3 => "ac3",
            Self::Wma => "asf",
        }
    }

    fn encoder_name(self) -> &'static str {
        match self {
            Self::M4a => "aac",
            Self::Ac3 => "ac3",
            Self::Wma => "wmav2",
        }
    }
}

pub fn handles_extension(ext: &str) -> bool {
    let ext = ext.strip_prefix('.').unwrap_or(ext).to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "m4a"
            | "aac"
            | "mp4"
            | "m4v"
            | "mov"
            | "3gp"
            | "mkv"
            | "webm"
            | "avi"
            | "wmv"
            | "wma"
            | "asf"
            | "ac3"
            | "eac3"
            | "flv"
            | "mpg"
            | "mpeg"
            | "ts"
            | "amr"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_FILE: AtomicUsize = AtomicUsize::new(0);

    fn temp_path(extension: &str) -> std::path::PathBuf {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "au-io-ffmpeg-{}-{sequence}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn export_import_round_trip() {
        let ffmpeg = match get() {
            Ok(ffmpeg) => ffmpeg,
            Err(error) => {
                eprintln!("Skipping FFmpeg round-trip tests: {error}");
                return;
            }
        };

        let sample_rate = 48_000_u32;
        let frames = sample_rate as usize;
        let left: Vec<f32> = (0..frames)
            .map(|index| {
                (std::f32::consts::TAU * 440.0 * index as f32 / sample_rate as f32).sin() * 0.5
            })
            .collect();
        let right: Vec<f32> = (0..frames)
            .map(|index| {
                (std::f32::consts::TAU * 660.0 * index as f32 / sample_rate as f32).sin() * 0.5
            })
            .collect();
        let original = AudioBuffer::from_channels(sample_rate, vec![left, right]);

        for format in ExportFormat::all() {
            let path = temp_path(format.extension());
            let export_result = ffmpeg.export(
                &path,
                &original,
                format,
                format.default_bit_rate(),
            );
            assert!(
                export_result.is_ok(),
                "export {} failed: {:?}",
                format.label(),
                export_result.err()
            );

            let imported = ffmpeg.import(&path);
            let _ = std::fs::remove_file(&path);
            let imported = imported.unwrap_or_else(|error| {
                panic!("import {} failed: {error:?}", format.label())
            });

            assert_eq!(imported.sample_rate, sample_rate);
            assert_eq!(imported.n_channels(), 2);
            assert!(
                imported.len().abs_diff(frames) <= (sample_rate as usize * 15) / 100,
                "{} length {} differs too much from {}",
                format.label(),
                imported.len(),
                frames
            );

            let channel = &imported.channels[0];
            let start = channel.len() / 4;
            let end = channel.len() * 3 / 4;
            let middle = &channel[start..end];
            let rms = (middle.iter().map(|sample| sample * sample).sum::<f32>()
                / middle.len() as f32)
                .sqrt();
            assert!(
                (rms - 0.3536).abs() <= 0.3536 * 0.25,
                "{} RMS was {rms}",
                format.label()
            );

            let crossings = middle
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();
            let duration = middle.len() as f32 / sample_rate as f32;
            let estimated_frequency = crossings as f32 / duration;
            assert!(
                (estimated_frequency - 440.0).abs() <= 440.0 * 0.05,
                "{} frequency estimate was {estimated_frequency}",
                format.label()
            );
        }
    }

    #[test]
    fn missing_message_is_actionable() {
        let message = missing_message("test loader error");
        assert!(message.contains("libavformat.so.61"));
        assert!(message.contains("apt install"));
        assert!(message.contains("WAV"));
    }

    #[test]
    fn load_from_empty_directory_fails() {
        let path = std::env::temp_dir().join(format!(
            "au-io-ffmpeg-empty-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        assert!(Ffmpeg::load_from(Some(&path)).is_err());
        let _ = std::fs::remove_dir(path);
    }
}
