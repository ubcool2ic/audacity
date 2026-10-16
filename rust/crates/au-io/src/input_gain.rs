use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Scales captured samples by a recording gain in the range `0.0..=1.0`.
///
/// Samples are not clipped. A unity gain leaves the buffer bit-identical.
pub fn apply_input_gain(buf: &mut [f32], gain: f32) {
    let gain = clamp_gain(gain);
    if gain == 1.0 {
        return;
    }
    if gain == 0.0 {
        buf.fill(0.0);
        return;
    }
    for sample in buf {
        *sample *= gain;
    }
}

fn clamp_gain(gain: f32) -> f32 {
    if gain.is_nan() {
        0.0
    } else {
        gain.clamp(0.0, 1.0)
    }
}

/// A lock-free, shareable software input-gain value.
#[derive(Clone, Debug)]
pub struct InputGain {
    inner: Arc<AtomicU32>,
}

impl InputGain {
    /// Creates a gain handle initialized to `gain`, clamped to `0.0..=1.0`.
    pub fn new(gain: f32) -> Self {
        Self {
            inner: Arc::new(AtomicU32::new(clamp_gain(gain).to_bits())),
        }
    }

    /// Sets the shared gain, clamped to `0.0..=1.0`.
    pub fn set(&self, gain: f32) {
        self.inner
            .store(clamp_gain(gain).to_bits(), Ordering::Relaxed);
    }

    /// Returns the current shared gain.
    pub fn get(&self) -> f32 {
        f32::from_bits(self.inner.load(Ordering::Relaxed))
    }
}

impl Default for InputGain {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Indicates whether recording gain is controlled by the OS mixer or software.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixerKind {
    /// Gain is controlled by an input mixer.
    Os,
    /// Gain is applied to captured samples in software.
    Software,
}

/// Interface for an operating-system input-volume mixer.
pub trait InputMixer: Send {
    /// Sets the mixer volume and returns whether the operation succeeded.
    fn set_volume(&mut self, v: f32) -> bool;

    /// Returns the current mixer volume when it can be read.
    fn volume(&self) -> Option<f32>;
}

/// A PulseAudio/PipeWire source mixer controlled through the `pactl` CLI.
pub struct PulseSourceMixer {
    source: String,
}

impl PulseSourceMixer {
    /// Detects a usable `pactl` source mixer.
    ///
    /// `source` is a PulseAudio source name; `None` means `@DEFAULT_SOURCE@`.
    /// Returns `None` if the platform is not Linux or the source volume cannot
    /// be queried successfully.
    pub fn detect(source: Option<&str>) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            let source = source.unwrap_or("@DEFAULT_SOURCE@").to_owned();
            let output = std::process::Command::new("pactl")
                .arg("get-source-volume")
                .arg(&source)
                .output()
                .ok()?;
            if output.status.success() {
                Some(Self { source })
            } else {
                None
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = source;
            None
        }
    }

    /// Detects the source mixer for a capture device as named by the audio
    /// backend (cpal/ALSA name or PulseAudio description). Falls back to
    /// `@DEFAULT_SOURCE@` for the generic `default`/`pulse`/`pipewire` devices
    /// or when no matching source exists.
    pub fn detect_for_device(device_name: Option<&str>) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            let specific = device_name.and_then(|name| {
                let out = std::process::Command::new("pactl")
                    .args(["list", "sources"])
                    .output()
                    .ok()?;
                if !out.status.success() {
                    return None;
                }
                pulse_source_for_device(&String::from_utf8_lossy(&out.stdout), name)
            });
            if let Some(src) = specific {
                if let Some(m) = Self::detect(Some(&src)) {
                    return Some(m);
                }
            }
            Self::detect(None)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = device_name;
            None
        }
    }

    /// The PulseAudio source this mixer controls.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Picks the PulseAudio source (from `pactl list sources` output) that
/// corresponds to a backend device name. Monitor sources are skipped.
/// Returns `None` for generic devices (`default`, `pulse`, `pipewire`, `sysdefault`)
/// so the caller uses `@DEFAULT_SOURCE@`.
pub fn pulse_source_for_device(pactl_sources: &str, device_name: &str) -> Option<String> {
    let dev = device_name.trim();
    let generic = ["", "default", "pulse", "pipewire", "sysdefault"];
    if generic.contains(&dev.to_ascii_lowercase().as_str()) {
        return None;
    }
    // ALSA names such as "hw:CARD=USB,DEV=0" or "sysdefault:CARD=PCH".
    let card = dev
        .split(|c| c == ':' || c == ',')
        .find_map(|part| part.strip_prefix("CARD="))
        .map(str::to_ascii_lowercase);
    let dev_lower = dev.to_ascii_lowercase();

    struct Src { name: String, desc: String, card_name: String, card_id: String }
    let mut sources: Vec<Src> = Vec::new();
    for line in pactl_sources.lines() {
        let t = line.trim();
        if t.starts_with("Source #") {
            sources.push(Src { name: String::new(), desc: String::new(), card_name: String::new(), card_id: String::new() });
        } else if let Some(cur) = sources.last_mut() {
            if let Some(v) = t.strip_prefix("Name:") {
                cur.name = v.trim().to_owned();
            } else if let Some(v) = t.strip_prefix("Description:") {
                cur.desc = v.trim().to_owned();
            } else if let Some(v) = t.strip_prefix("alsa.card_name = ") {
                cur.card_name = v.trim().trim_matches('"').to_owned();
            } else if let Some(v) = t.strip_prefix("alsa.id = ") {
                cur.card_id = v.trim().trim_matches('"').to_owned();
            }
        }
    }
    let inputs = sources.iter().filter(|s| !s.name.is_empty() && !s.name.ends_with(".monitor"));
    let mut best: Option<&Src> = None;
    for s in inputs {
        if s.name == dev || s.desc == dev {
            return Some(s.name.clone());
        }
        let name_l = s.name.to_ascii_lowercase();
        let matched = match &card {
            Some(c) => name_l.contains(c.as_str())
                || s.card_name.to_ascii_lowercase() == *c
                || s.card_id.to_ascii_lowercase() == *c,
            None => !s.desc.is_empty()
                && (s.desc.to_ascii_lowercase().contains(&dev_lower)
                    || dev_lower.contains(&s.desc.to_ascii_lowercase())),
        };
        if matched && best.is_none() {
            best = Some(s);
        }
    }
    best.map(|s| s.name.clone())
}

/// Opens the system mixer for a capture device: WASAPI endpoint volume on
/// Windows, the CoreAudio input volume scalar on macOS, the per-device
/// PulseAudio/PipeWire source on Linux. `None` means no system control is
/// available and recording gain must be applied in software.
pub fn system_input_mixer(device_name: Option<&str>) -> Option<Box<dyn InputMixer>> {
    #[cfg(windows)]
    {
        return WasapiInputMixer::detect(device_name).map(|m| Box::new(m) as Box<dyn InputMixer>);
    }
    #[cfg(target_os = "macos")]
    {
        return CoreAudioInputMixer::detect(device_name).map(|m| Box::new(m) as Box<dyn InputMixer>);
    }
    #[cfg(target_os = "linux")]
    {
        return PulseSourceMixer::detect_for_device(device_name).map(|m| Box::new(m) as Box<dyn InputMixer>);
    }
    #[allow(unreachable_code)]
    {
        let _ = device_name;
        None
    }
}

impl InputMixer for PulseSourceMixer {
    fn set_volume(&mut self, v: f32) -> bool {
        let percent = (clamp_gain(v) * 100.0).round() as u32;
        std::process::Command::new("pactl")
            .arg("set-source-volume")
            .arg(&self.source)
            .arg(format!("{percent}%"))
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn volume(&self) -> Option<f32> {
        let output = std::process::Command::new("pactl")
            .arg("get-source-volume")
            .arg(&self.source)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_pactl_volume(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Parses the first percentage in `pactl get-source-volume` output as a
/// fraction, such that `50%` becomes `0.5`.
pub fn parse_pactl_volume(s: &str) -> Option<f32> {
    for (percent_index, character) in s.char_indices() {
        if character != '%' {
            continue;
        }

        let mut start = percent_index;
        while start > 0 {
            let byte = s.as_bytes()[start - 1];
            if byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-') {
                start -= 1;
            } else {
                break;
            }
        }

        if start == percent_index {
            continue;
        }

        if let Ok(percent) = s[start..percent_index].parse::<f32>() {
            if percent.is_finite() {
                return Some(percent / 100.0);
            }
        }
    }
    None
}

/// WASAPI system-mixer input gain (`IAudioEndpointVolume` on the capture endpoint).
#[cfg(windows)]
mod wasapi {
    use super::InputMixer;

    // ============================================================================
    // Windows (WASAPI)
    // ============================================================================

    fn ensure_com() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
    }

    pub struct WasapiInputMixer {
        endpoint_volume: windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume,
    }

    // Safety: IAudioEndpointVolume is an agile MTA COM object, safe to send across threads;
    // ensure_com() initializes MTA on the calling thread before any interface method invocation.
    unsafe impl Send for WasapiInputMixer {}

    impl WasapiInputMixer {
        pub fn detect(device_name: Option<&str>) -> Option<Self> {
            ensure_com();

            let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator = unsafe {
                windows::Win32::System::Com::CoCreateInstance(
                    &windows::Win32::Media::Audio::MMDeviceEnumerator,
                    None,
                    windows::Win32::System::Com::CLSCTX_ALL,
                )
            }
            .ok()?;

            let device = if let Some(target_name) = device_name {
                let collection = unsafe {
                    enumerator.EnumAudioEndpoints(
                        windows::Win32::Media::Audio::eCapture,
                        windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE,
                    )
                }
                .ok()?;

                let count = unsafe { collection.GetCount() }.ok()?;
                let mut matched = None;

                for i in 0..count {
                    let dev = match unsafe { collection.Item(i) } {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    let store = match unsafe {
                        dev.OpenPropertyStore(windows::Win32::System::Com::STGM_READ)
                    } {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let prop = match unsafe {
                        store.GetValue(&windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName)
                    } {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let pwstr = match unsafe {
                        windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc(&prop)
                    } {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    let name = unsafe {
                        let mut len = 0;
                        while *pwstr.0.add(len) != 0 {
                            len += 1;
                        }
                        let slice = core::slice::from_raw_parts(pwstr.0, len);
                        String::from_utf16_lossy(slice)
                    };
                    unsafe {
                        windows::Win32::System::Com::CoTaskMemFree(Some(pwstr.0 as *const core::ffi::c_void));
                    }

                    if name == target_name {
                        matched = Some(dev);
                        break;
                    }
                }
                matched?
            } else {
                unsafe {
                    enumerator.GetDefaultAudioEndpoint(
                        windows::Win32::Media::Audio::eCapture,
                        windows::Win32::Media::Audio::eConsole,
                    )
                }
                .ok()?
            };

            let endpoint_volume: windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume =
                unsafe { device.Activate(windows::Win32::System::Com::CLSCTX_ALL, None) }.ok()?;

            unsafe { endpoint_volume.GetMasterVolumeLevelScalar() }.ok()?;

            Some(Self { endpoint_volume })
        }
    }

    impl InputMixer for WasapiInputMixer {
        fn set_volume(&mut self, v: f32) -> bool {
            ensure_com();
            unsafe {
                self.endpoint_volume
                    .SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), core::ptr::null())
                    .is_ok()
            }
        }

        fn volume(&self) -> Option<f32> {
            ensure_com();
            unsafe { self.endpoint_volume.GetMasterVolumeLevelScalar().ok() }
        }

    }
}
#[cfg(windows)]
pub use wasapi::WasapiInputMixer;

/// CoreAudio system-mixer input gain (`kAudioDevicePropertyVolumeScalar`, input scope).
#[cfg(target_os = "macos")]
#[allow(non_upper_case_globals, non_snake_case)]
mod coreaudio {
    use super::InputMixer;
    use std::ffi::c_void;
    use std::mem::size_of;

    #[repr(C)]
    #[derive(Copy, Clone, Debug)]
    struct AudioObjectPropertyAddress {
        mSelector: u32,
        mScope: u32,
        mElement: u32,
    }

    type OSStatus = i32;
    type CFStringRef = *const c_void;

    const kAudioObjectSystemObject: u32 = 1;
    const kAudioHardwarePropertyDefaultInputDevice: u32 = u32::from_be_bytes(*b"dIn ");
    const kAudioHardwarePropertyDevices: u32 = u32::from_be_bytes(*b"dev#");
    const kAudioObjectPropertyName: u32 = u32::from_be_bytes(*b"lnam");
    const kAudioDevicePropertyScopeInput: u32 = u32::from_be_bytes(*b"inpt");
    const kAudioObjectPropertyScopeGlobal: u32 = u32::from_be_bytes(*b"glob");
    const kAudioDevicePropertyVolumeScalar: u32 = u32::from_be_bytes(*b"volm");
    const kCFStringEncodingUTF8: u32 = 0x08000100;

    #[link(name = "CoreAudio", kind = "framework")]
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(
            inObjectID: u32,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const c_void,
            outDataSize: *mut u32,
        ) -> OSStatus;

        fn AudioObjectGetPropertyData(
            inObjectID: u32,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const c_void,
            ioDataSize: *mut u32,
            outData: *mut c_void,
        ) -> OSStatus;

        fn AudioObjectSetPropertyData(
            inObjectID: u32,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const c_void,
            inDataSize: u32,
            inData: *const c_void,
        ) -> OSStatus;

        fn AudioObjectHasProperty(
            inObjectID: u32,
            inAddress: *const AudioObjectPropertyAddress,
        ) -> u8;

        fn AudioObjectIsPropertySettable(
            inObjectID: u32,
            inAddress: *const AudioObjectPropertyAddress,
            outIsSettable: *mut u8,
        ) -> OSStatus;

        fn CFStringGetCString(
            theString: CFStringRef,
            buffer: *mut u8,
            bufferSize: isize,
            encoding: u32,
        ) -> u8;

        fn CFRelease(cf: *const c_void);
    }

    pub struct CoreAudioInputMixer {
        device: u32,
        elements: Vec<u32>,
    }

    impl CoreAudioInputMixer {
        fn is_volume_settable(device: u32, element: u32) -> bool {
            let addr = AudioObjectPropertyAddress {
                mSelector: kAudioDevicePropertyVolumeScalar,
                mScope: kAudioDevicePropertyScopeInput,
                mElement: element,
            };
            unsafe {
                if AudioObjectHasProperty(device, &addr) == 0 {
                    return false;
                }
                let mut settable: u8 = 0;
                AudioObjectIsPropertySettable(device, &addr, &mut settable) == 0 && settable != 0
            }
        }

        fn get_device_name(device: u32) -> Option<String> {
            let addr = AudioObjectPropertyAddress {
                mSelector: kAudioObjectPropertyName,
                mScope: kAudioObjectPropertyScopeGlobal,
                mElement: 0,
            };
            unsafe {
                let mut cf_str: CFStringRef = std::ptr::null();
                let mut size = size_of::<CFStringRef>() as u32;
                if AudioObjectGetPropertyData(device, &addr, 0, std::ptr::null(), &mut size, &mut cf_str as *mut _ as *mut _) != 0 || cf_str.is_null() {
                    return None;
                }
                let mut buf = [0u8; 256];
                let res = CFStringGetCString(cf_str, buf.as_mut_ptr(), buf.len() as isize, kCFStringEncodingUTF8);
                CFRelease(cf_str);
                if res == 0 {
                    return None;
                }
                let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                std::str::from_utf8(&buf[..len]).ok().map(|s| s.to_string())
            }
        }

        pub fn detect(device_name: Option<&str>) -> Option<Self> {
            let mut target_device = None;
            if let Some(name) = device_name {
                let addr = AudioObjectPropertyAddress {
                    mSelector: kAudioHardwarePropertyDevices,
                    mScope: kAudioObjectPropertyScopeGlobal,
                    mElement: 0,
                };
                unsafe {
                    let mut size = 0u32;
                    if AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &addr, 0, std::ptr::null(), &mut size) == 0 && size > 0 {
                        let count = size as usize / size_of::<u32>();
                        let mut devices = vec![0u32; count];
                        if AudioObjectGetPropertyData(kAudioObjectSystemObject, &addr, 0, std::ptr::null(), &mut size, devices.as_mut_ptr() as *mut _) == 0 {
                            for dev in devices {
                                if let Some(dev_name) = Self::get_device_name(dev) {
                                    if dev_name == name {
                                        target_device = Some(dev);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let device = match target_device {
                Some(dev) => dev,
                None => {
                    let addr = AudioObjectPropertyAddress {
                        mSelector: kAudioHardwarePropertyDefaultInputDevice,
                        mScope: kAudioObjectPropertyScopeGlobal,
                        mElement: 0,
                    };
                    let mut dev = 0u32;
                    let mut size = size_of::<u32>() as u32;
                    unsafe {
                        if AudioObjectGetPropertyData(kAudioObjectSystemObject, &addr, 0, std::ptr::null(), &mut size, &mut dev as *mut _ as *mut _) != 0 || dev == 0 {
                            return None;
                        }
                    }
                    dev
                }
            };

            let mut elements = Vec::new();
            if Self::is_volume_settable(device, 0) {
                elements.push(0);
            } else {
                for elem in 1..=2 {
                    if Self::is_volume_settable(device, elem) {
                        elements.push(elem);
                    }
                }
            }

            if elements.is_empty() {
                None
            } else {
                Some(Self { device, elements })
            }
        }
    }

    impl InputMixer for CoreAudioInputMixer {
        fn set_volume(&mut self, v: f32) -> bool {
            let val = v.clamp(0.0, 1.0);
            let mut ok = true;
            for &elem in &self.elements {
                let addr = AudioObjectPropertyAddress {
                    mSelector: kAudioDevicePropertyVolumeScalar,
                    mScope: kAudioDevicePropertyScopeInput,
                    mElement: elem,
                };
                let status = unsafe {
                    AudioObjectSetPropertyData(
                        self.device,
                        &addr,
                        0,
                        std::ptr::null(),
                        size_of::<f32>() as u32,
                        &val as *const _ as *const _,
                    )
                };
                if status != 0 {
                    ok = false;
                }
            }
            ok
        }

        fn volume(&self) -> Option<f32> {
            let mut sum = 0.0;
            let mut count = 0;
            for &elem in &self.elements {
                let addr = AudioObjectPropertyAddress {
                    mSelector: kAudioDevicePropertyVolumeScalar,
                    mScope: kAudioDevicePropertyScopeInput,
                    mElement: elem,
                };
                let mut val = 0.0f32;
                let mut size = size_of::<f32>() as u32;
                let status = unsafe {
                    AudioObjectGetPropertyData(
                        self.device,
                        &addr,
                        0,
                        std::ptr::null(),
                        &mut size,
                        &mut val as *mut _ as *mut _,
                    )
                };
                if status == 0 {
                    sum += val;
                    count += 1;
                }
            }
            if count > 0 {
                Some(sum / count as f32)
            } else {
                None
            }
        }
    }
}
#[cfg(target_os = "macos")]
pub use coreaudio::CoreAudioInputMixer;

/// Coordinates OS mixer control and software input-gain fallback.
pub struct RecordVolume {
    gain: InputGain,
    mixer: Option<Box<dyn InputMixer>>,
    last_value: f32,
}

impl RecordVolume {
    /// Creates recording-volume control, initially set to unity gain.
    pub fn new(mixer: Option<Box<dyn InputMixer>>) -> Self {
        Self {
            gain: InputGain::default(),
            mixer,
            last_value: 1.0,
        }
    }

    /// Returns the active mixer kind.
    pub fn kind(&self) -> MixerKind {
        if self.mixer.is_some() {
            MixerKind::Os
        } else {
            MixerKind::Software
        }
    }

    /// Sets recording volume, falling back permanently to software if the
    /// mixer is absent or cannot set the requested value.
    pub fn set(&mut self, v: f32) {
        let v = clamp_gain(v);
        self.last_value = v;

        if let Some(mixer) = self.mixer.as_mut() {
            if mixer.set_volume(v) {
                self.gain.set(1.0);
                return;
            }
            self.mixer = None;
        }

        self.gain.set(v);
    }

    /// Returns a cloneable gain handle for use by the audio callback.
    pub fn software_gain(&self) -> InputGain {
        self.gain.clone()
    }

    /// Swaps in the mixer for a newly selected capture device (or `None` for
    /// software gain), resetting software gain to unity when a mixer is used.
    pub fn set_mixer(&mut self, mixer: Option<Box<dyn InputMixer>>) {
        self.mixer = mixer;
        if self.mixer.is_some() {
            self.gain.set(1.0);
        } else {
            self.gain.set(self.last_value);
        }
    }

    /// Picks up a change made outside the app (e.g. in the system mixer).
    /// Returns the new volume when the OS mixer value moved away from the last
    /// value this controller set or reported; `None` otherwise or in software mode.
    pub fn poll_external(&mut self) -> Option<f32> {
        let v = clamp_gain(self.mixer.as_ref()?.volume()?);
        if (v - self.last_value).abs() > 0.005 {
            self.last_value = v;
            Some(v)
        } else {
            None
        }
    }

    /// Returns the readable OS mixer volume, or the last value set when it
    /// cannot be read.
    pub fn get(&self) -> f32 {
        self.mixer
            .as_ref()
            .and_then(|mixer| mixer.volume())
            .unwrap_or(self.last_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn unity_gain_is_bit_identical() {
        let mut samples = [0.0, -0.0, 0.25, -1.5, f32::INFINITY, f32::NAN];
        let original: Vec<u32> = samples.iter().map(|sample| sample.to_bits()).collect();

        apply_input_gain(&mut samples, 1.0);

        let result: Vec<u32> = samples.iter().map(|sample| sample.to_bits()).collect();
        assert_eq!(result, original);
    }

    #[test]
    fn half_gain_scales_samples() {
        let mut samples = [2.0, -1.0, 0.5];
        apply_input_gain(&mut samples, 0.5);
        assert_eq!(samples, [1.0, -0.5, 0.25]);
    }

    #[test]
    fn zero_gain_silences_samples() {
        let mut samples = [1.0, -2.0, f32::NAN, f32::INFINITY];
        apply_input_gain(&mut samples, 0.0);
        assert_eq!(samples, [0.0; 4]);
    }

    #[test]
    fn gain_is_clamped() {
        let mut above = [0.5];
        apply_input_gain(&mut above, 2.0);
        assert_eq!(above, [0.5]);

        let mut below = [0.5];
        apply_input_gain(&mut below, -1.0);
        assert_eq!(below, [0.0]);

        assert_eq!(InputGain::new(2.0).get(), 1.0);
        assert_eq!(InputGain::new(-1.0).get(), 0.0);
    }

    #[test]
    fn input_gain_is_shared_across_clones_and_threads() {
        let gain = InputGain::default();
        let cloned = gain.clone();

        let thread = thread::spawn(move || cloned.set(0.375));
        thread.join().unwrap();

        assert_eq!(gain.get(), 0.375);
    }

    #[test]
    fn parses_pactl_volume() {
        let output = "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB\n        balance 0.00";
        assert_eq!(parse_pactl_volume(output), Some(0.5));
        assert_eq!(parse_pactl_volume("garbage"), None);
    }

    struct MockMixer {
        succeeds: bool,
        current: f32,
    }

    impl InputMixer for MockMixer {
        fn set_volume(&mut self, v: f32) -> bool {
            if self.succeeds {
                self.current = v;
                true
            } else {
                false
            }
        }

        fn volume(&self) -> Option<f32> {
            Some(self.current)
        }
    }

    #[test]
    fn record_volume_uses_successful_mixer() {
        let mixer = MockMixer {
            succeeds: true,
            current: 1.0,
        };
        let mut volume = RecordVolume::new(Some(Box::new(mixer)));

        volume.set(0.4);

        assert_eq!(volume.kind(), MixerKind::Os);
        assert_eq!(volume.software_gain().get(), 1.0);
        assert_eq!(volume.get(), 0.4);
    }

    #[test]
    fn record_volume_falls_back_after_mixer_failure() {
        let mixer = MockMixer {
            succeeds: false,
            current: 1.0,
        };
        let mut volume = RecordVolume::new(Some(Box::new(mixer)));

        volume.set(0.4);

        assert_eq!(volume.kind(), MixerKind::Software);
        assert_eq!(volume.software_gain().get(), 0.4);
        assert_eq!(volume.get(), 0.4);
    }

    #[test]
    fn record_volume_without_mixer_uses_software() {
        let mut volume = RecordVolume::new(None);

        assert_eq!(volume.kind(), MixerKind::Software);
        assert_eq!(volume.get(), 1.0);

        volume.set(0.25);

        assert_eq!(volume.software_gain().get(), 0.25);
        assert_eq!(volume.get(), 0.25);
    }
    #[test]
    fn poll_external_detects_system_mixer_changes() {
        use std::sync::{Arc, Mutex};
        struct Shared(Arc<Mutex<f32>>);
        impl InputMixer for Shared {
            fn set_volume(&mut self, v: f32) -> bool { *self.0.lock().unwrap() = v; true }
            fn volume(&self) -> Option<f32> { Some(*self.0.lock().unwrap()) }
        }
        let level = Arc::new(Mutex::new(1.0));
        let mut volume = RecordVolume::new(Some(Box::new(Shared(level.clone()))));
        volume.set(0.5);
        assert_eq!(volume.poll_external(), None);
        *level.lock().unwrap() = 0.8;
        assert_eq!(volume.poll_external(), Some(0.8));
        assert_eq!(volume.poll_external(), None);
        volume.set_mixer(None);
        assert_eq!(volume.kind(), MixerKind::Software);
        assert_eq!(volume.software_gain().get(), 0.8);
        assert_eq!(volume.poll_external(), None);
    }

    #[test]
    fn maps_backend_device_to_pulse_source() {
        let out = "Source #1\n\tName: alsa_output.pci-0000_00_1f.3.analog-stereo.monitor\n\tDescription: Monitor of Built-in Audio\nSource #2\n\tName: alsa_input.pci-0000_00_1f.3.analog-stereo\n\tDescription: Built-in Audio Analog Stereo\n\tProperties:\n\t\talsa.card_name = \"HDA Intel PCH\"\n\t\talsa.id = \"PCH\"\nSource #3\n\tName: alsa_input.usb-Blue_Yeti-00.analog-stereo\n\tDescription: Yeti Stereo Microphone Analog Stereo\n";
        assert_eq!(pulse_source_for_device(out, "default"), None);
        assert_eq!(pulse_source_for_device(out, "Yeti Stereo Microphone Analog Stereo").as_deref(), Some("alsa_input.usb-Blue_Yeti-00.analog-stereo"));
        assert_eq!(pulse_source_for_device(out, "Yeti Stereo Microphone").as_deref(), Some("alsa_input.usb-Blue_Yeti-00.analog-stereo"));
        assert_eq!(pulse_source_for_device(out, "hw:CARD=PCH,DEV=0").as_deref(), Some("alsa_input.pci-0000_00_1f.3.analog-stereo"));
        assert_eq!(pulse_source_for_device(out, "hw:CARD=Nope,DEV=0"), None);
    }
}
