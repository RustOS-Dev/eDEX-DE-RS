//! Volume on RustOS: ALSA's mixer controls (`/dev/snd/controlC*`, the Linux sound drivers in
//! RustOS images), else the OSS mixer through RustOS's `mixer` tool (its native drivers).

use std::{fs::File, os::fd::AsRawFd, path::Path};

use anyhow::{anyhow, Context, Result};

use crate::{
    audio::{AudioDevice, AudioState},
    runner::CommandRunner,
};

// ─── ALSA control interface (include/uapi/sound/asound.h) ───────────────────

const NAME_LEN: usize = 44;
const IFACE_MIXER: i32 = 2;
const TYPE_BOOLEAN: i32 = 1;
const TYPE_INTEGER: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct ElemId {
    numid: u32,
    iface: i32,
    device: u32,
    subdevice: u32,
    name: [u8; NAME_LEN],
    index: u32,
}

#[repr(C)]
struct ElemList {
    offset: u32,
    space: u32,
    used: u32,
    count: u32,
    pids: *mut ElemId,
    reserved: [u8; 50],
}

#[repr(C)]
struct ElemInfo {
    id: ElemId,
    kind: i32,
    access: u32,
    count: u32,
    owner: i32,
    /// `integer.{min,max,step}` for integer controls.
    value: [i64; 16],
    reserved: [u8; 64],
}

#[repr(C)]
struct ElemValue {
    id: ElemId,
    indirect: u32,
    _pad: u32,
    value: [i64; 128],
    reserved: [u8; 128],
}

const _: () = {
    assert!(std::mem::size_of::<ElemId>() == 64);
    assert!(std::mem::size_of::<ElemList>() == 80);
    assert!(std::mem::size_of::<ElemInfo>() == 272);
    assert!(std::mem::size_of::<ElemValue>() == 1224);
};

const IOCTL_ELEM_LIST: libc::c_ulong = 0xc050_5510;
const IOCTL_ELEM_INFO: libc::c_ulong = 0xc110_5511;
const IOCTL_ELEM_READ: libc::c_ulong = 0xc4c8_5512;
const IOCTL_ELEM_WRITE: libc::c_ulong = 0xc4c8_5513;

fn zeroed<T>() -> T {
    // SAFETY: only used for the plain-data ioctl structs above.
    unsafe { std::mem::zeroed() }
}

fn ioctl<T>(f: &File, req: libc::c_ulong, arg: &mut T) -> Result<()> {
    // SAFETY: `arg` is the struct the request expects, sized as asserted above.
    let r = unsafe { libc::ioctl(f.as_raw_fd(), req as _, arg as *mut T) };
    if r < 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}

fn id_name(id: &ElemId) -> String {
    let end = id.name.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    String::from_utf8_lossy(&id.name[..end]).into_owned()
}

/// One card's control device.
pub struct Card {
    file: File,
    ids: Vec<ElemId>,
}

/// A volume control with its optional switch.
struct Control {
    volume: Option<ElemId>,
    switch: Option<ElemId>,
}

impl Card {
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut list = ElemList {
            offset: 0,
            space: 0,
            used: 0,
            count: 0,
            pids: std::ptr::null_mut(),
            reserved: [0; 50],
        };
        ioctl(&file, IOCTL_ELEM_LIST, &mut list)?;
        let mut ids = vec![zeroed::<ElemId>(); list.count as usize];
        list.space = list.count;
        list.pids = ids.as_mut_ptr();
        ioctl(&file, IOCTL_ELEM_LIST, &mut list)?;
        ids.truncate(list.used as usize);
        ids.retain(|i| i.iface == IFACE_MIXER);
        Ok(Self { file, ids })
    }

    pub fn control_names(&self) -> Vec<String> {
        self.ids.iter().map(id_name).collect()
    }

    fn find(&self, name: &str) -> Option<ElemId> {
        self.ids.iter().find(|i| id_name(i) == name).copied()
    }

    /// The first of `bases` that has "<base> <dir> Volume" or a switch.
    fn control(&self, bases: &[&str], dir: &str) -> Option<Control> {
        bases.iter().find_map(|b| {
            let volume = self.find(&format!("{b} {dir} Volume"));
            let switch = self.find(&format!("{b} {dir} Switch"));
            (volume.is_some() || switch.is_some()).then_some(Control { volume, switch })
        })
    }

    fn info(&self, id: ElemId) -> Result<ElemInfo> {
        let mut info: ElemInfo = zeroed();
        info.id = id;
        ioctl(&self.file, IOCTL_ELEM_INFO, &mut info)?;
        Ok(info)
    }

    fn read(&self, id: ElemId) -> Result<ElemValue> {
        let mut v: ElemValue = zeroed();
        v.id = id;
        ioctl(&self.file, IOCTL_ELEM_READ, &mut v)?;
        Ok(v)
    }

    fn write(&self, id: ElemId, values: &[i64]) -> Result<()> {
        let mut v: ElemValue = zeroed();
        v.id = id;
        let n = values.len().min(128);
        v.value[..n].copy_from_slice(&values[..n]);
        ioctl(&self.file, IOCTL_ELEM_WRITE, &mut v)
    }

    /// Volume in percent (channel average) of an integer control.
    fn percent(&self, id: ElemId) -> Result<u32> {
        let info = self.info(id)?;
        if info.kind != TYPE_INTEGER {
            return Err(anyhow!("{} is not an integer control", id_name(&id)));
        }
        let (min, max) = (info.value[0], info.value[1]);
        let v = self.read(id)?;
        let n = (info.count as usize).clamp(1, 128);
        let avg = v.value[..n].iter().sum::<i64>() as f64 / n as f64;
        Ok(scale_to_percent(avg, min, max))
    }

    fn set_percent(&self, id: ElemId, percent: u32) -> Result<()> {
        let info = self.info(id)?;
        let (min, max) = (info.value[0], info.value[1]);
        let n = (info.count as usize).clamp(1, 128);
        let raw = percent_to_scale(percent, min, max);
        self.write(id, &vec![raw; n])
    }

    /// Whether the switch is on (sound passes).
    fn switch_on(&self, id: ElemId) -> Result<bool> {
        let info = self.info(id)?;
        if info.kind != TYPE_BOOLEAN {
            return Err(anyhow!("{} is not a switch", id_name(&id)));
        }
        let n = (info.count as usize).clamp(1, 128);
        Ok(self.read(id)?.value[..n].iter().any(|&v| v != 0))
    }

    fn set_switch(&self, id: ElemId, on: bool) -> Result<()> {
        let n = (self.info(id)?.count as usize).clamp(1, 128);
        self.write(id, &vec![on as i64; n])
    }
}

fn scale_to_percent(v: f64, min: i64, max: i64) -> u32 {
    if max <= min {
        return 0;
    }
    (((v - min as f64) * 100.0 / (max - min) as f64).round()).clamp(0.0, 100.0) as u32
}

fn percent_to_scale(p: u32, min: i64, max: i64) -> i64 {
    min + ((max - min) as f64 * p.min(100) as f64 / 100.0).round() as i64
}

const PLAYBACK: [&str; 6] = ["Master", "PCM", "Speaker", "Headphone", "Front", "Line Out"];
const CAPTURE: [&str; 4] = ["Capture", "Mic", "Internal Mic", "Front Mic"];

/// `/proc/asound/cards`: (number, "Name - long name").
pub fn parse_cards(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|l| {
            let (num, rest) = l.trim_start().split_once(' ')?;
            let num: u32 = num.parse().ok()?;
            let desc = rest.split_once("]:").map(|(_, d)| d.trim()).unwrap_or("");
            Some((num, desc.to_string()))
        })
        .collect()
}

fn cards() -> Vec<(u32, String)> {
    parse_cards(&std::fs::read_to_string("/proc/asound/cards").unwrap_or_default())
}

/// The first card with a playback volume.
fn default_card() -> Option<(Card, Control)> {
    for (n, _) in cards() {
        let Ok(card) = Card::open(Path::new(&format!("/dev/snd/controlC{n}"))) else {
            continue;
        };
        if let Some(c) = card.control(&PLAYBACK, "Playback") {
            return Some((card, c));
        }
    }
    None
}

fn alsa_query() -> Option<AudioState> {
    let (card, out) = default_card()?;
    let mut st = AudioState {
        available: true,
        ..Default::default()
    };
    st.volume = out.volume.and_then(|v| card.percent(v).ok()).unwrap_or(100);
    st.muted = out
        .switch
        .and_then(|s| card.switch_on(s).ok())
        .map(|on| !on)
        .unwrap_or(false);
    if let Some(cap) = card.control(&CAPTURE, "Capture") {
        st.mic_volume = cap.volume.and_then(|v| card.percent(v).ok()).unwrap_or(100);
        st.mic_muted = cap
            .switch
            .and_then(|s| card.switch_on(s).ok())
            .map(|on| !on)
            .unwrap_or(false);
    }
    // Cards are the devices; the first with a volume control is the default.
    let mut first = true;
    for (n, desc) in cards() {
        let dev = AudioDevice {
            id: n,
            name: desc,
            default: std::mem::take(&mut first),
        };
        st.sinks.push(dev);
    }
    Some(st)
}

// ─── OSS fallback (`mixer`) ─────────────────────────────────────────────────

/// `mixer` output: "volume  80% 80%".
fn parse_mixer(text: &str, which: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        (w.next()? == which).then_some(())?;
        let left: u32 = w.next()?.trim_end_matches('%').parse().ok()?;
        let right: u32 = w
            .next()
            .and_then(|r| r.trim_end_matches('%').parse().ok())
            .unwrap_or(left);
        Some((left + right) / 2)
    })
}

fn oss_query(r: &dyn CommandRunner) -> AudioState {
    let mut st = AudioState::default();
    if let Ok(out) = r.run("mixer", &[]) {
        if let Some(v) = out
            .ok()
            .then(|| parse_mixer(&out.stdout, "volume"))
            .flatten()
        {
            st.available = true;
            st.volume = v;
            // OSS has no mute switch: a muted card is at 0.
            st.muted = v == 0;
        }
    }
    st
}

fn oss_set(r: &dyn CommandRunner, percent: u32) -> Result<()> {
    r.run_ok("mixer", &["volume", &percent.min(100).to_string()])
        .map(|_| ())
}

// ─── Requests ───────────────────────────────────────────────────────────────

pub fn query(r: &dyn CommandRunner) -> AudioState {
    alsa_query().unwrap_or_else(|| oss_query(r))
}

pub fn set_volume(r: &dyn CommandRunner, percent: u32) -> Result<()> {
    match default_card() {
        Some((
            card,
            Control {
                volume: Some(v), ..
            },
        )) => card.set_percent(v, percent.min(100)),
        Some(_) => Err(anyhow!("the sound card has no volume control")),
        None => oss_set(r, percent),
    }
}

pub fn adjust_volume(r: &dyn CommandRunner, delta: i32) -> Result<()> {
    let now = query(r).volume as i32;
    set_volume(r, (now + delta).clamp(0, 100) as u32)
}

/// Remembered across OSS "mutes" (the volume goes to 0 and comes back).
static OSS_UNMUTED: std::sync::Mutex<u32> = std::sync::Mutex::new(80);

pub fn toggle_mute(r: &dyn CommandRunner) -> Result<()> {
    match default_card() {
        Some((
            card,
            Control {
                switch: Some(s), ..
            },
        )) => {
            let on = card.switch_on(s)?;
            card.set_switch(s, !on)
        }
        Some(_) => Err(anyhow!("the sound card has no mute switch")),
        None => {
            let st = oss_query(r);
            if st.volume > 0 {
                *OSS_UNMUTED.lock().unwrap() = st.volume;
                oss_set(r, 0)
            } else {
                let v = *OSS_UNMUTED.lock().unwrap();
                oss_set(r, v)
            }
        }
    }
}

pub fn toggle_mic_mute(_r: &dyn CommandRunner) -> Result<()> {
    let (card, _) = default_card().ok_or_else(|| anyhow!("no ALSA sound card"))?;
    let cap = card
        .control(&CAPTURE, "Capture")
        .and_then(|c| c.switch)
        .ok_or_else(|| anyhow!("no capture switch"))?;
    let on = card.switch_on(cap)?;
    card.set_switch(cap, !on)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_proc_asound_cards() {
        let text = " 0 [Intel          ]: HDA-Intel - HDA Intel\n                      HDA Intel at 0xfebf0000 irq 33\n 1 [Audio          ]: USB-Audio - QEMU USB Audio\n                      QEMU QEMU USB Audio at usb-0000:00:04.0-1, full speed\n";
        let cards = parse_cards(text);
        assert_eq!(
            cards,
            vec![
                (0, "HDA-Intel - HDA Intel".to_string()),
                (1, "USB-Audio - QEMU USB Audio".to_string())
            ]
        );
    }

    #[test]
    fn scales_volume() {
        assert_eq!(scale_to_percent(0.0, 0, 87), 0);
        assert_eq!(scale_to_percent(87.0, 0, 87), 100);
        assert_eq!(scale_to_percent(43.5, 0, 87), 50);
        assert_eq!(percent_to_scale(50, 0, 87), 44);
        assert_eq!(percent_to_scale(100, -64, 0), 0);
        assert_eq!(percent_to_scale(0, -64, 0), -64);
    }

    #[test]
    fn oss_mixer_output() {
        let r = FakeRunner::default().with("mixer", "volume  64% 66%\npcm     100% 100%\n");
        let st = oss_query(&r);
        assert!(st.available);
        assert_eq!(st.volume, 65);
        assert!(!st.muted);
        assert_eq!(parse_mixer("pcm 10% 20%\n", "pcm"), Some(15));
    }
}
