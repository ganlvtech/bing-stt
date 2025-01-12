use std::ptr::null_mut;
use std::slice;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{eCapture, eConsole, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX, WAVEFORMATEXTENSIBLE};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitialize, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, STGM_READ};

struct ComReleaser;

impl ComReleaser {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            CoInitialize(None).ok()?;
            Ok(Self {})
        }
    }
}

impl Drop for ComReleaser {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct CoTaskMemReleaser(*mut std::ffi::c_void);

impl Drop for CoTaskMemReleaser {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0));
        }
    }
}

#[derive(Clone)]
pub enum WaveFormat {
    WAVEFORMATEX(WAVEFORMATEX),
    WAVEFORMATEXTENSIBLE(WAVEFORMATEXTENSIBLE),
}

impl WaveFormat {
    pub fn as_bytes<'a>(&self) -> &'a [u8] {
        match self {
            WaveFormat::WAVEFORMATEX(wfx) => {
                let size = size_of::<WAVEFORMATEX>();
                let ptr = wfx as *const WAVEFORMATEX as *const u8;
                unsafe { slice::from_raw_parts(ptr, size) }
            }
            WaveFormat::WAVEFORMATEXTENSIBLE(wfxe) => {
                let size = size_of::<WAVEFORMATEXTENSIBLE>();
                let ptr = wfxe as *const WAVEFORMATEXTENSIBLE as *const u8;
                unsafe { slice::from_raw_parts(ptr, size) }
            }
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            WaveFormat::WAVEFORMATEX(wfx) => {
                let size = size_of::<WAVEFORMATEX>();
                let ptr = wfx as *const WAVEFORMATEX as *const u8;
                unsafe { slice::from_raw_parts(ptr, size - 2).to_vec() }
            }
            WaveFormat::WAVEFORMATEXTENSIBLE(wfxe) => {
                let mut wfx = wfxe.Format.clone();
                wfx.wFormatTag = WAVE_FORMAT_IEEE_FLOAT as _;
                let size = size_of::<WAVEFORMATEX>();
                let ptr = &wfx as *const WAVEFORMATEX as *const u8;
                unsafe { slice::from_raw_parts(ptr, size - 2).to_vec() }
            }
        }
    }
}

impl AsRef<WAVEFORMATEX> for WaveFormat {
    fn as_ref(&self) -> &WAVEFORMATEX {
        match self {
            WaveFormat::WAVEFORMATEX(wfx) => wfx,
            WaveFormat::WAVEFORMATEXTENSIBLE(wfxe) => &wfxe.Format,
        }
    }
}

const REFTIMES_PER_SEC: i64 = 10000000;

pub struct Recorder {
    _com_releaser: ComReleaser,
    _enumerator: IMMDeviceEnumerator,
    device: IMMDevice,
    audio_client: IAudioClient,
    wave_format: WaveFormat,
    capture_client: IAudioCaptureClient,
}

impl Recorder {
    pub fn new() -> windows::core::Result<Recorder> {
        unsafe {
            // 代码改造自 MSDN 的示例代码
            // https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
            // https://learn.microsoft.com/en-us/windows/win32/coreaudio/capturing-a-stream
            // https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize#examples
            let com_releaser = ComReleaser::new()?; // CoInitialize 是引用计数的，CoUninitialize 需要与 CoInitialize 个数一致。
            // 说明：windows-rs 封装了所有 IXxx 接口，底层的 IUnknown 实现了 Drop，会自动释放，不需要我们手动调用 Release
            // 但是我们必须将他保存在结构体的字段中，否则，离开当前函数就会被立刻释放了。
            let enumerator = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?; // 麦克风录制使用这一行代码
            // let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?; // Windows 内录则使用这一行代码
            let audio_client = device.Activate::<IAudioClient>(CLSCTX_ALL, None)?;
            let pwfx = audio_client.GetMixFormat()?;
            let _pwfx_releaser = CoTaskMemReleaser(pwfx as _);
            let wave_format = if (*pwfx).wFormatTag == WAVE_FORMAT_EXTENSIBLE as u16 {
                let pwfxe = pwfx as *mut WAVEFORMATEXTENSIBLE;
                WaveFormat::WAVEFORMATEXTENSIBLE(*pwfxe)
            } else {
                WaveFormat::WAVEFORMATEX(*pwfx)
            };
            audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, REFTIMES_PER_SEC, 0, pwfx, None)?; // 麦克风录制使用这一行代码
            // audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, REFTIMES_PER_SEC, 0, pwfx, None)?; // Windows 内录则使用这一行代码
            let capture_client = audio_client.GetService::<IAudioCaptureClient>()?;
            audio_client.Start()?;
            Ok(Self {
                _com_releaser: com_releaser,
                _enumerator: enumerator,
                device,
                audio_client,
                wave_format,
                capture_client,
            })
        }
    }

    pub fn device_name(&self) -> windows::core::Result<String> {
        unsafe {
            let property_store = self.device.OpenPropertyStore(STGM_READ)?;
            let device_name = property_store.GetValue(&PKEY_Device_FriendlyName)?;
            Ok(device_name.to_string())
        }
    }

    pub fn wave_format(&self) -> &WaveFormat {
        &self.wave_format
    }

    pub fn capture(&mut self, mut handler: impl FnMut(&[u8])) -> windows::core::Result<u32> {
        unsafe {
            let mut p_data: *mut u8 = null_mut();
            let mut num_frames_to_read = 0;
            let mut dwflags = 0;
            self.capture_client.GetBuffer(&mut p_data, &mut num_frames_to_read, &mut dwflags, None, None)?;
            if num_frames_to_read > 0 {
                let block_align = self.wave_format.as_ref().nBlockAlign as usize;
                let data = slice::from_raw_parts(p_data, num_frames_to_read as usize * block_align);
                handler(data);
                self.capture_client.ReleaseBuffer(num_frames_to_read)?;
            }
            Ok(num_frames_to_read)
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.audio_client.Stop();
        }
    }
}
