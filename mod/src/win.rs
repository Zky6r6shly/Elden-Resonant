//! Everything that runs inside Elden Ring (Windows only).

use std::collections::HashSet;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use eldenring::cs::{CSTaskGroupIndex, CSTaskImp, CharaInitParam, EquipParamGoods, EquipParamWeapon, SoloParamRepository, WorldChrMan};
use eldenring::fd4::FD4TaskData;
use fromsoftware_shared::{FromStatic, SharedTaskImpExt};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Memory::{MEM_COMMIT, MEM_PRIVATE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, PAGE_READWRITE, VirtualAlloc, VirtualQuery};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::core::w;

use crate::config::{self, Config};
use crate::generated::{ABILITIES, CUE_ARRIVAL, CUES, FINISHERS, FORMS, START_CLASS};
use crate::{cotr, cues, fmg, locate, traversal};

static MODULE: AtomicUsize = AtomicUsize::new(0);
static CR_FOUND: AtomicBool = AtomicBool::new(false);
static LOG: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
static AUDIO: OnceLock<Mutex<Sender<usize>>> = OnceLock::new();
static FREE_POWERS: AtomicBool = AtomicBool::new(false);

fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("EldenResonant")
}

pub fn log(msg: &str) {
    let m = LOG.get_or_init(|| {
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        Mutex::new(std::fs::File::create(dir.join("elden-resonant.log")).ok())
    });
    if let Ok(mut g) = m.lock() {
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{msg}");
            let _ = f.flush();
        }
    }
}

fn dll_dir() -> PathBuf {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(HMODULE(MODULE.load(Ordering::Relaxed) as _)), &mut buf) } as usize;
    PathBuf::from(String::from_utf16_lossy(&buf[..n])).parent().map(Path::to_path_buf).unwrap_or_default()
}

fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Valve\\Steam"), w!("SteamPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut size))
    };
    if ok.is_ok() {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        roots.push(PathBuf::from(String::from_utf16_lossy(&buf[..n]).replace('/', "\\")));
    }
    for p in ["C:\\Program Files (x86)\\Steam", "C:\\Program Files\\Steam"] {
        let p = PathBuf::from(p);
        if !roots.contains(&p) {
            roots.push(p);
        }
    }
    roots
}

/// # Safety
/// Called by the Windows loader when ModEngine2 loads this DLL.
#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub unsafe extern "system" fn DllMain(hmodule: usize, reason: u32, _: usize) -> bool {
    if reason == 1 {
        MODULE.store(hmodule, Ordering::Relaxed);
        std::panic::set_hook(Box::new(|info| log(&format!("error: {info}"))));
        std::thread::spawn(|| {
            // a failure in the mod must never take the game down with it
            if std::panic::catch_unwind(run).is_err() {
                log("stopped after an error (see above); the game keeps running without the mashup's live features");
            }
        });
    }
    true
}

fn run() {
    log(&format!("Elden Resonant {} starting", env!("CARGO_PKG_VERSION")));
    let cfg = std::fs::read_to_string(dll_dir().join("elden-resonant.ini")).map(|t| config::parse(&t)).unwrap_or_default();
    FREE_POWERS.store(cfg.free_powers, Ordering::Relaxed);
    log(&format!("free powers (no FP cost): {} (F8 toggles)", if cfg.free_powers { "on" } else { "off" }));
    let explicit = std::env::var("ELDEN_RESONANT_CR_PATH").ok().or(cfg.cr_path.clone()).map(PathBuf::from);
    let epic = PathBuf::from("C:\\ProgramData\\Epic\\EpicGamesLauncher\\Data\\Manifests");
    let cr = locate::find(explicit, &steam_roots(), &epic);
    match &cr {
        Some(p) => {
            log(&format!("CONTROL Resonant found at {}", p.display()));
            CR_FOUND.store(true, Ordering::Relaxed);
            let (tx, rx) = channel();
            let _ = AUDIO.set(Mutex::new(tx));
            let (p, cfg) = (p.clone(), cfg.clone());
            std::thread::spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| audio_main(p, cfg, rx))).is_err() {
                    log("audio stopped after an error");
                }
            });
        }
        None => log("CONTROL Resonant not found; the Aberrant's descriptions will say so in game"),
    }

    if std::env::var_os("ELDEN_RESONANT_SELFTEST").is_some() {
        start_text_pass("selftest");
    }

    // Params and text only need the game's singletons, so they work on any
    // game version; poll for them from this thread.
    std::thread::spawn(|| {
        let r = std::panic::catch_unwind(|| {
            while !unsafe { apply_params() } {
                std::thread::sleep(Duration::from_millis(250));
            }
            log("params applied");
            // text tables can finish loading after the params; patching is idempotent
            for (wait, why) in [(0, "boot"), (20, "boot+20s"), (60, "boot+60s")] {
                std::thread::sleep(Duration::from_secs(wait));
                start_text_pass(why);
            }
        });
        if r.is_err() {
            log("could not change Elden Ring's items; this game version may not be supported");
        }
    });

    // The per-frame hook (sound cues) needs the library's addresses for this exact game version.
    let Ok(task) = CSTaskImp::wait_for_instance(Duration::MAX) else {
        log("game task runner not found; sound cues are off");
        return;
    };
    let mut state = State::default();
    let mut failed = false;
    task.run_recurring(
        move |_: &FD4TaskData| {
            if !failed && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.frame())).is_err() {
                failed = true;
                log("frame hook disabled after an error");
            }
        },
        CSTaskGroupIndex::FrameBegin,
    );
}

fn play(cue: usize) {
    if let Some(tx) = AUDIO.get() {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(cue);
        }
    }
}

#[derive(Default)]
struct State {
    arrived: bool,
    last_fp: Option<i32>,
    last_weapon: Option<u32>,
    last_frame: Option<Instant>,
    f8_down: bool,
    traversal: traversal::Traversal,
}

impl State {
    fn frame(&mut self) {
        let Ok(wcm) = (unsafe { WorldChrMan::instance_mut() }) else { return };
        let now = Instant::now();
        let dt = self.last_frame.map(|t| (now - t).as_secs_f32()).unwrap_or(0.0).min(0.1);
        self.last_frame = Some(now);

        // F8 toggles free powers
        let f8 = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x77) } < 0;
        if f8 && !self.f8_down {
            let on = !FREE_POWERS.load(Ordering::Relaxed);
            FREE_POWERS.store(on, Ordering::Relaxed);
            log(&format!("free powers {}", if on { "on" } else { "off" }));
        }
        self.f8_down = f8;

        let Some(player) = wcm.main_player.as_mut() else {
            self.last_fp = None;
            self.last_weapon = None;
            self.traversal = traversal::Traversal::default();
            return;
        };
        if !self.arrived {
            self.arrived = true;
            play(CUE_ARRIVAL);
            // text tables for later content (DLC) can load after boot
            start_text_pass("in world");
        }

        let mut fp = player.chr_ins.modules.data.fp;
        let pgd = unsafe { player.player_game_data.as_ref() };
        let asm = &pgd.equipment.chr_asm;
        let slot = asm.equipment.selected_slots.right_weapon_slot.min(2) as usize;
        let wep = asm.equipment_param_ids[1 + 2 * slot];
        let base = if wep > 0 { (wep as u32 / 10000) * 10000 } else { 0 };
        let form = FORMS.iter().position(|f| f.weapon_id == base);

        if self.last_weapon.is_some() && self.last_weapon != Some(base) {
            if let Some(f) = form {
                play(FORMS[f].cue);
            }
        }
        self.last_weapon = Some(base);

        if let Some(prev) = self.last_fp {
            let delta = prev - fp;
            if delta > 0 {
                if let Some(a) = cues::ability_for_fp_drop(delta) {
                    play(ABILITIES[a].cue);
                } else if let Some(f) = form {
                    play(FINISHERS[FORMS[f].finisher].cue);
                }
            }
        }

        // Elevate, Double Jump, Rush
        let modules = &mut player.chr_ins.modules;
        let req = &modules.action_request;
        let input = traversal::Input {
            dt,
            airborne: !modules.physics.standing_on_solid_ground,
            riding: modules.ride.ride_node.ride_state != 0,
            jump_pressed: req.new_action_presses.jump(),
            jump_held: req.action_requests.jump(),
            jump_held_for: req.action_timers.jump,
            sprinting: req.movement_request_flags.dash(),
            fp,
        };
        let out = self.traversal.frame(input);
        if let Some(g) = out.gravity {
            modules.physics.gravity_multiplier = g;
        }
        if let Some(m) = out.motion {
            modules.physics.motion_multiplier = m;
        }
        if out.reset_fall {
            modules.fall.fall_timer = 0.0;
        }
        if let Some(c) = out.cue {
            play(c);
        }
        if out.fp_spent > 0 {
            fp = (fp - out.fp_spent).max(0);
        }
        if FREE_POWERS.load(Ordering::Relaxed) {
            fp = modules.data.max_fp;
        }
        if fp != modules.data.fp {
            modules.data.fp = fp;
        }
        self.last_fp = Some(fp);
    }
}

/// Returns false until the regulation is loaded.
unsafe fn apply_params() -> bool {
    let Ok(repo) = (unsafe { SoloParamRepository::instance_mut() }) else { return false };
    if repo.get::<EquipParamWeapon>(FORMS[0].weapon_id).is_none() || repo.get::<CharaInitParam>(START_CLASS.chara_init_id).is_none() {
        return false;
    }
    for f in FORMS {
        // the base row and its infusions (+100 per affinity)
        for k in 0..=12u32 {
            let Some(w) = repo.get_mut::<EquipParamWeapon>(f.weapon_id + k * 100) else { continue };
            w.set_attack_base_physics(f.attack_physics);
            w.set_weight(f.weight);
            w.set_proper_strength(0);
            w.set_proper_agility(0);
            w.set_proper_magic(0);
            w.set_proper_faith(0);
            w.set_proper_luck(0);
            w.set_sword_arts_param_id(FINISHERS[f.finisher].sword_arts_id);
        }
    }
    for a in ABILITIES {
        if let Some(g) = repo.get_mut::<EquipParamGoods>(a.goods_id) {
            g.set_is_consume(false);
            g.set_consume_mp(a.fp_cost);
        }
    }
    if let Some(c) = repo.get_mut::<CharaInitParam>(START_CLASS.chara_init_id) {
        for f in FORMS {
            let id = f.weapon_id as i32;
            match f.class_slot {
                "R1" => c.set_equip_wep_right(id),
                "R2" => c.set_equip_subwep_right(id),
                "R3" => c.set_equip_subwep_right3(id),
                "L1" => c.set_equip_wep_left(id),
                "L2" => c.set_equip_subwep_left(id),
                "L3" => c.set_equip_subwep_left3(id),
                _ => {}
            }
        }
        for a in ABILITIES {
            let id = a.goods_id as i32;
            match a.class_item_slot {
                1 => { c.set_item_01(id); c.set_item_num_01(1) }
                2 => { c.set_item_02(id); c.set_item_num_02(1) }
                3 => { c.set_item_03(id); c.set_item_num_03(1) }
                4 => { c.set_item_04(id); c.set_item_num_04(1) }
                5 => { c.set_item_05(id); c.set_item_num_05(1) }
                6 => { c.set_item_06(id); c.set_item_num_06(1) }
                7 => { c.set_item_07(id); c.set_item_num_07(1) }
                8 => { c.set_item_08(id); c.set_item_num_08(1) }
                9 => { c.set_item_09(id); c.set_item_num_09(1) }
                10 => { c.set_item_10(id); c.set_item_num_10(1) }
                _ => {}
            }
        }
        let s = &START_CLASS;
        c.set_soul_lv(s.soul_lv);
        c.set_base_vit(s.vit);
        c.set_base_wil(s.mind);
        c.set_base_end(s.end);
        c.set_base_str(s.str_);
        c.set_base_dex(s.dex);
        c.set_base_mag(s.int);
        c.set_base_fai(s.fai);
        c.set_base_luc(s.arc);
    }
    true
}

// ---- text: find FMG tables in memory and repoint our ids' strings ----

static PATCHED: OnceLock<Mutex<HashSet<(usize, usize)>>> = OnceLock::new();
static ARENAS: OnceLock<Mutex<Vec<(usize, usize, usize)>>> = OnceLock::new(); // (base, used, size)

fn start_text_pass(why: &'static str) {
    std::thread::spawn(move || {
        let t = Instant::now();
        let Ok((tables, patched)) = std::panic::catch_unwind(|| unsafe { text_pass() }) else { return };
        log(&format!("text pass ({why}): {tables} FMG tables seen, {patched} strings replaced in {:?}", t.elapsed()));
    });
}

unsafe fn read(addr: usize, buf: &mut [u8]) -> usize {
    let mut n = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), addr as _, buf.as_mut_ptr() as _, buf.len(), Some(&mut n)) };
    if ok.is_ok() { n } else { 0 }
}

/// Store a UTF-16 string somewhere above `fmg_base` and return its address.
unsafe fn store_string(fmg_base: usize, text: &str) -> Option<usize> {
    let mut data: Vec<u8> = text.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    data.extend_from_slice(&[0, 0]);
    let mut arenas = ARENAS.get_or_init(Default::default).lock().ok()?;
    let fits = |&(b, used, size): &(usize, usize, usize)| b > fmg_base && used + data.len() <= size;
    let i = match arenas.iter().position(fits) {
        Some(i) => i,
        None => {
            const SIZE: usize = 0x10000;
            let mut hint = (fmg_base & !0xFFFF) + 0x10000;
            let mut got = 0usize;
            for _ in 0..16384 {
                let p = unsafe { VirtualAlloc(Some(hint as _), SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) } as usize;
                if p > fmg_base {
                    got = p;
                    break;
                }
                hint += 0x10000;
            }
            if got == 0 {
                return None;
            }
            arenas.push((got, 0, SIZE));
            arenas.len() - 1
        }
    };
    let (b, used, _) = arenas[i];
    let at = b + used;
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), at as *mut u8, data.len()) };
    arenas[i].1 += (data.len() + 7) & !7;
    Some(at)
}

unsafe fn patch(base: usize, f: &fmg::Fmg, id: Option<i32>, index: usize, text: &str) -> bool {
    let key = (base, index);
    let patched = PATCHED.get_or_init(Default::default);
    if patched.lock().map(|p| p.contains(&key)).unwrap_or(true) {
        return false;
    }
    let Some(at) = (unsafe { store_string(base, text) }) else {
        log(&format!("no memory above FMG 0x{base:x} for id {id:?}"));
        return false;
    };
    let off = (at - base) as i64;
    let slot = base + f.offset_slot(index);
    let ok = unsafe { WriteProcessMemory(GetCurrentProcess(), slot as _, off.to_le_bytes().as_ptr() as _, 8, None) }.is_ok();
    if ok {
        patched.lock().unwrap().insert(key);
    }
    ok
}

unsafe fn text_pass() -> (usize, usize) {
    const WINDOW: usize = 16 << 20;
    let cr_found = CR_FOUND.load(Ordering::Relaxed);
    let own: Vec<(usize, usize)> = ARENAS.get_or_init(Default::default).lock().map(|a| a.iter().map(|x| (x.0, x.2)).collect()).unwrap_or_default();
    let (mut tables, mut patched) = (0, 0);
    let mut addr = 0x10000usize;
    let mut buf = vec![0u8; WINDOW];
    loop {
        let mut mbi = MEMORY_BASIC_INFORMATION::default();
        if unsafe { VirtualQuery(Some(addr as _), &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
            break;
        }
        let (rbase, rsize) = (mbi.BaseAddress as usize, mbi.RegionSize);
        let readable = mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE && (mbi.Protect.0 & (PAGE_GUARD.0 | PAGE_NOACCESS.0)) == 0 && mbi.Protect.0 != 0;
        if readable && !own.iter().any(|&(b, _)| b == rbase) {
            let mut pos = 0usize;
            while pos + fmg::HEADER <= rsize {
                let n = unsafe { read(rbase + pos, &mut buf[..WINDOW.min(rsize - pos)]) };
                if n < fmg::HEADER {
                    break;
                }
                let mut i = 0usize;
                let mut skip_to = 0usize;
                while i + fmg::HEADER <= n {
                    if fmg::looks_like_header(&buf[i..]) {
                        let size = u32::from_le_bytes(buf[i + 4..i + 8].try_into().unwrap()) as usize;
                        let mut whole;
                        let bytes: &[u8] = if i + size <= n {
                            &buf[i..n]
                        } else if size <= 64 << 20 && pos + i + size <= rsize {
                            whole = vec![0u8; size];
                            if unsafe { read(rbase + pos + i, &mut whole) } != size {
                                i += 4;
                                continue;
                            }
                            &whole
                        } else {
                            i += 4;
                            continue;
                        };
                        if let Some(f) = fmg::Fmg::parse(bytes) {
                            tables += 1;
                            let fbase = rbase + pos + i;
                            if let Some(t) = fmg::table_of(&f) {
                                for (id, text) in fmg::plan(t, f.kind(), cr_found) {
                                    if let Some(ix) = f.index_of(id) {
                                        patched += unsafe { patch(fbase, &f, Some(id), ix, &text) } as usize;
                                    }
                                }
                            }
                            for ix in 0..f.string_count as usize {
                                if f.string_at_index(ix).as_deref() == Some(START_CLASS.replaces_class_text) {
                                    patched += unsafe { patch(fbase, &f, None, ix, START_CLASS.class_name) } as usize;
                                }
                            }
                            let len = f.bytes.len();
                            i += (len + 3) & !3;
                            skip_to = skip_to.max(i);
                            continue;
                        }
                    }
                    i += 4;
                }
                // overlap windows so a header near the end is seen whole next time
                let step = if n == WINDOW { (n - 0x1000).max(skip_to) } else { n };
                pos += step & !3;
            }
        }
        addr = rbase.saturating_add(rsize);
        if addr >= 0x7FFF_FFFF_0000 {
            break;
        }
    }
    (tables, patched)
}

// ---- audio: CONTROL Resonant's own sounds ----

fn hash(s: &str, n: u64) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes().chain(n.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

fn decode(vgm: &Path, wem: &Path, wav: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(vgm)
        .arg("-o")
        .arg(wav)
        .arg(wem)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && wav.is_file()
}

fn audio_main(cr: PathBuf, cfg: Config, rx: Receiver<usize>) {
    let t = Instant::now();
    let mut archives = Vec::new();
    let mut all: Vec<(usize, usize)> = Vec::new();
    let mut paths = Vec::new();
    for toc in cotr::find_tocs(&cr, 4) {
        match cotr::open(&toc) {
            Ok(a) => {
                let ai = archives.len();
                for (ei, e) in a.entries.iter().enumerate() {
                    all.push((ai, ei));
                    paths.push(e.path.clone());
                }
                archives.push(a);
            }
            Err(e) => log(&format!("skipping {}: {e}", toc.display())),
        }
    }
    let wems = paths.iter().filter(|p| p.to_lowercase().ends_with(".wem")).count();
    log(&format!("CONTROL Resonant: {} archives, {} files, {wems} sounds indexed in {:?}", archives.len(), paths.len(), t.elapsed()));

    let cache = data_dir().join("cache");
    let _ = std::fs::create_dir_all(&cache);
    let vgm = dll_dir().join("vgmstream").join("vgmstream-cli.exe");
    let choice = cues::choose(&paths, &cfg.pinned);
    let mut sounds: Vec<Option<Arc<[u8]>>> = vec![None; CUES.len()];
    for (ci, cue) in CUES.iter().enumerate() {
        let Some(pi) = choice[ci] else {
            log(&format!("cue {}: no matching sound", cue.key));
            continue;
        };
        let (ai, ei) = all[pi];
        let entry = &archives[ai].entries[ei];
        let stem = format!("{}-{:016x}", cue.key, hash(&entry.path, entry.size as u64));
        let wav = cache.join(format!("{stem}.wav"));
        if !wav.is_file() {
            let wem = cache.join(format!("{stem}.wem"));
            match archives[ai].extract(entry) {
                Ok(bytes) => {
                    let _ = std::fs::write(&wem, bytes);
                }
                Err(e) => {
                    log(&format!("cue {}: could not read {}: {e}", cue.key, entry.path));
                    continue;
                }
            }
            let ok = decode(&vgm, &wem, &wav);
            let _ = std::fs::remove_file(&wem);
            if !ok {
                log(&format!("cue {}: vgmstream could not decode {}", cue.key, entry.path));
                continue;
            }
        }
        if let Ok(b) = std::fs::read(&wav) {
            log(&format!("cue {} -> {}", cue.key, entry.path));
            sounds[ci] = Some(b.into());
        }
    }
    drop(archives);

    let Ok(mut sink) = rodio::DeviceSinkBuilder::open_default_sink() else {
        log("no audio output device");
        return;
    };
    sink.log_on_drop(false);
    let master = cfg.volume.unwrap_or(0.8);
    let ready = Instant::now();
    let mut last_play = vec![Instant::now() - Duration::from_secs(60); CUES.len()];
    for cue in rx {
        // drop requests that piled up while sounds were decoding, except the arrival theme
        if ready.elapsed() < Duration::from_millis(50) && cue != CUE_ARRIVAL {
            continue;
        }
        if last_play[cue].elapsed() < Duration::from_millis(400) {
            continue;
        }
        let Some(bytes) = sounds[cue].clone() else { continue };
        let Ok(dec) = rodio::Decoder::new(Cursor::new(bytes)) else { continue };
        let p = rodio::Player::connect_new(sink.mixer());
        p.set_volume((master * CUES[cue].volume) as _);
        p.append(dec);
        p.detach();
        last_play[cue] = Instant::now();
    }
}
