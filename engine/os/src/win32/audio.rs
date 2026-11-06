//! Audio por WASAPI, em modo compartilhado: o mixer do Windows mistura nossa
//! saida com a dos outros programas e converte para o formato do
//! dispositivo.
//!
//! Sem evento nem thread: `audio_queued` le quanto falta tocar
//! (`GetCurrentPadding`) e `audio_write` copia direto para o buffer do
//! dispositivo (`GetBuffer`/`ReleaseBuffer`).

use super::{Win32, ffi};
use crate::audio::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE, Audio, AudioHandle};
use crate::memory::Memory;
use core::{mem, ptr};

// A mascara de canais em `format` e a de estereo.
const _: () = assert!(AUDIO_CHANNELS == 2);

/// Estado da saida, no heap; `AudioHandle` aponta para ca.
struct AudioState {
    client: *mut ffi::IAudioClient,
    render: *mut ffi::IAudioRenderClient,
    capacity: u32,
    /// Esta abertura iniciou o COM na thread e precisa encerrar no fim.
    com_initialized: bool,
}

#[inline]
fn state(audio: &AudioHandle) -> *mut AudioState {
    audio.0.cast()
}

/// `HRESULT` negativo e falha; `S_OK` e `S_FALSE` sao sucesso.
#[inline]
fn failed(result: ffi::HRESULT) -> bool {
    result < 0
}

/// `Release` de qualquer interface: toda vtable comeca com a de `IUnknown`.
/// Nulo e ignorado.
unsafe fn release<T>(object: *mut T) {
    if object.is_null() {
        return;
    }
    unsafe {
        let unknown = object as *mut ffi::IUnknown;
        ((*(*unknown).lpVtbl).Release)(unknown);
    }
}

/// `f32` estereo na taxa do contrato.
fn format() -> ffi::WAVEFORMATEXTENSIBLE {
    let block = (AUDIO_CHANNELS * mem::size_of::<f32>()) as ffi::WORD;
    ffi::WAVEFORMATEXTENSIBLE {
        Format: ffi::WAVEFORMATEX {
            wFormatTag: ffi::WAVE_FORMAT_EXTENSIBLE,
            nChannels: AUDIO_CHANNELS as ffi::WORD,
            nSamplesPerSec: AUDIO_SAMPLE_RATE,
            nAvgBytesPerSec: AUDIO_SAMPLE_RATE * block as ffi::DWORD,
            nBlockAlign: block,
            wBitsPerSample: 32,
            // Bytes depois do `WAVEFORMATEX`.
            cbSize: (mem::size_of::<ffi::WAVEFORMATEXTENSIBLE>() - mem::size_of::<ffi::WAVEFORMATEX>()) as ffi::WORD,
        },
        wValidBitsPerSample: 32,
        dwChannelMask: ffi::SPEAKER_FRONT_LEFT | ffi::SPEAKER_FRONT_RIGHT,
        SubFormat: ffi::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
    }
}

/// Dispositivo padrao -> `IAudioClient` iniciado e tocando, com o
/// `IAudioRenderClient` dele e o tamanho da fila. Cada passo depende do
/// anterior; o que for criado e nao sobrar e liberado.
unsafe fn open_client(
    latency_ms: u32,
) -> Option<(*mut ffi::IAudioClient, *mut ffi::IAudioRenderClient, u32)> {
    let mut enumerator: *mut ffi::IMMDeviceEnumerator = ptr::null_mut();
    let mut device: *mut ffi::IMMDevice = ptr::null_mut();
    let mut client: *mut ffi::IAudioClient = ptr::null_mut();
    let mut render: *mut ffi::IAudioRenderClient = ptr::null_mut();
    let mut capacity: ffi::UINT32 = 0;

    let ok = unsafe {
        'open: {
            if failed(ffi::CoCreateInstance(
                &ffi::CLSID_MMDeviceEnumerator,
                ptr::null_mut(),
                ffi::CLSCTX_ALL,
                &ffi::IID_IMMDeviceEnumerator,
                (&raw mut enumerator).cast(),
            )) {
                break 'open false;
            }
            let get_default = (*(*enumerator).lpVtbl).GetDefaultAudioEndpoint;
            if failed(get_default(enumerator, ffi::eRender, ffi::eConsole, &mut device)) {
                break 'open false;
            }
            let activate = (*(*device).lpVtbl).Activate;
            if failed(activate(
                device,
                &ffi::IID_IAudioClient,
                ffi::CLSCTX_ALL,
                ptr::null_mut(),
                (&raw mut client).cast(),
            )) {
                break 'open false;
            }

            let vtable = &*(*client).lpVtbl;
            let format = format();
            let duration = latency_ms as ffi::REFERENCE_TIME * 10_000;
            if failed((vtable.Initialize)(
                client,
                ffi::AUDCLNT_SHAREMODE_SHARED,
                ffi::AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | ffi::AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                duration,
                0,
                (&raw const format).cast(),
                ptr::null(),
            )) {
                break 'open false;
            }
            if failed((vtable.GetBufferSize)(client, &mut capacity)) {
                break 'open false;
            }
            if failed((vtable.GetService)(client, &ffi::IID_IAudioRenderClient, (&raw mut render).cast())) {
                break 'open false;
            }
            // Com a fila vazia, o mixer toca silencio.
            if failed((vtable.Start)(client)) {
                break 'open false;
            }
            true
        }
    };

    unsafe {
        release(device);
        release(enumerator);
        if !ok {
            release(render);
            release(client);
            return None;
        }
    }
    Some((client, render, capacity))
}

impl Audio for Win32 {
    /// O COM e iniciado nesta thread no modelo multithread. Se ela ja tinha
    /// COM em outro modelo (`RPC_E_CHANGED_MODE`), o WASAPI funciona igual;
    /// so nao e esta abertura que encerra.
    fn audio_open(latency_ms: u32) -> Option<AudioHandle> {
        unsafe {
            let result = ffi::CoInitializeEx(ptr::null_mut(), ffi::COINIT_MULTITHREADED);
            let com_initialized = result == ffi::S_OK || result == ffi::S_FALSE;
            if !com_initialized && result != ffi::RPC_E_CHANGED_MODE {
                return None;
            }

            let state = Win32::heap_alloc(mem::size_of::<AudioState>()) as *mut AudioState;
            let opened = if state.is_null() { None } else { open_client(latency_ms) };
            let Some((client, render, capacity)) = opened else {
                Win32::heap_free(state.cast());
                if com_initialized {
                    ffi::CoUninitialize();
                }
                return None;
            };
            state.write(AudioState { client, render, capacity, com_initialized });
            Some(AudioHandle(state.cast()))
        }
    }

    #[inline]
    fn audio_capacity(audio: &AudioHandle) -> u32 {
        unsafe { (*state(audio)).capacity }
    }

    fn audio_queued(audio: &AudioHandle) -> Option<u32> {
        unsafe {
            let client = (*state(audio)).client;
            let mut padding: ffi::UINT32 = 0;
            if failed(((*(*client).lpVtbl).GetCurrentPadding)(client, &mut padding)) {
                return None;
            }
            Some(padding)
        }
    }

    /// Copia em bytes: o contrato nao garante alinhamento de `f32` no buffer
    /// do dispositivo.
    fn audio_write(audio: &AudioHandle, samples: &[f32]) -> bool {
        let frames = (samples.len() / AUDIO_CHANNELS) as ffi::UINT32;
        if frames == 0 {
            return true;
        }
        unsafe {
            let render = (*state(audio)).render;
            let vtable = &*(*render).lpVtbl;
            let mut data: *mut ffi::BYTE = ptr::null_mut();
            if failed((vtable.GetBuffer)(render, frames, &mut data)) {
                return false;
            }
            let bytes = frames as usize * AUDIO_CHANNELS * mem::size_of::<f32>();
            ptr::copy_nonoverlapping(samples.as_ptr().cast::<u8>(), data, bytes);
            !failed((vtable.ReleaseBuffer)(render, frames, 0))
        }
    }

    fn audio_close(audio: AudioHandle) {
        unsafe {
            let state = state(&audio);
            let AudioState { client, render, com_initialized, .. } = state.read();
            ((*(*client).lpVtbl).Stop)(client);
            release(render);
            release(client);
            Win32::heap_free(state.cast());
            if com_initialized {
                ffi::CoUninitialize();
            }
        }
    }
}
