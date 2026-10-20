use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Cursor};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use core_model::{Region, System};

use crate::color::{linear_to_srgb, srgb8_to_linear};
use crate::library::LibraryEntry;
use crate::worker::lock;

const REQUEST_CAPACITY: usize = 16;
const COMPLETION_CAPACITY: usize = 2;
const MAX_TIER: u32 = 2048;
const MAX_DECODE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtworkKey {
    pub system: System,
    pub sha1: String,
    pub release: Option<Region>,
    pub tier: u32,
}

impl ArtworkKey {
    pub fn new(entry: &LibraryEntry, release: Option<Region>, max_dimension: u32) -> Self {
        Self {
            system: entry.system,
            sha1: entry.sha1.clone(),
            release,
            tier: max_dimension.clamp(64, MAX_TIER),
        }
    }

    pub fn same_release(&self, other: &Self) -> bool {
        self.system == other.system && self.sha1 == other.sha1 && self.release == other.release
    }
}

#[derive(Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub mip_levels: Vec<MipLevel>,
    pub border_average: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverAccent {
    pub srgb: [f32; 3],
    pub linear: [f32; 3],
}

impl Default for CoverAccent {
    fn default() -> Self {
        let rgb = [118_u8, 118, 128];

        Self {
            srgb: rgb.map(|channel| f32::from(channel) / 255.0),
            linear: rgb.map(srgb8_to_linear),
        }
    }
}

#[derive(Debug)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    pub fn levels(&self) -> impl Iterator<Item = (u32, u32, &[u8])> {
        std::iter::once((self.width, self.height, self.rgba.as_slice())).chain(
            self.mip_levels
                .iter()
                .map(|mip| (mip.width, mip.height, mip.rgba.as_slice())),
        )
    }

    pub fn byte_len(&self) -> usize {
        self.levels().map(|(_, _, rgba)| rgba.len()).sum()
    }
}

#[derive(Debug, Default)]
pub struct DecodedArtwork {
    pub title: String,
    pub accent: CoverAccent,
    pub front: Option<DecodedImage>,
    pub back: Option<DecodedImage>,
    pub spine: Option<DecodedImage>,
    pub label: Option<DecodedImage>,
    pub label_top: Option<DecodedImage>,
    pub warnings: Vec<String>,
}

impl DecodedArtwork {
    pub fn byte_len(&self) -> usize {
        [
            &self.front,
            &self.back,
            &self.spine,
            &self.label,
            &self.label_top,
        ]
        .into_iter()
        .flatten()
        .map(DecodedImage::byte_len)
        .sum::<usize>()
            + self.title.len()
            + self.warnings.iter().map(String::len).sum::<usize>()
    }
}

pub struct ArtworkCompletion {
    pub key: ArtworkKey,
    pub result: Result<(), String>,
}

struct Request {
    generation: u64,
    key: ArtworkKey,
}

struct Completed {
    generation: u64,
    key: ArtworkKey,
    result: Result<DecodedArtwork, String>,
}

#[derive(Default)]
struct Mailbox {
    generation: u64,
    requests: VecDeque<Request>,
    completed: VecDeque<Completed>,
    active: usize,
    stale_discarded: u64,
    shutdown: bool,
}

struct Cached {
    artwork: Arc<DecodedArtwork>,
    used_at: u64,
}

pub struct ArtworkCache {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    resident: HashMap<ArtworkKey, Cached>,
    pending: HashSet<ArtworkKey>,
    failed: HashMap<ArtworkKey, Instant>,
    generation: u64,
    tick: u64,
    bytes: usize,
    budget: usize,
}

impl ArtworkCache {
    pub fn retry_failed(&mut self) {
        self.failed.clear();
        self.resident
            .retain(|_, cached| cached.artwork.front.is_some());
        self.bytes = self
            .resident
            .values()
            .map(|cached| cached.artwork.byte_len())
            .sum();
    }

    pub fn new(cpu_budget_bytes: usize, workers: usize) -> io::Result<Self> {
        let cache = Self {
            mailbox: Arc::new((Mutex::new(Mailbox::default()), Condvar::new())),
            resident: HashMap::new(),
            pending: HashSet::new(),
            failed: HashMap::new(),
            generation: 0,
            tick: 0,
            bytes: 0,
            budget: cpu_budget_bytes,
        };

        for index in 0..workers.clamp(1, 2) {
            let mailbox = cache.mailbox.clone();

            std::thread::Builder::new()
                .name(format!("launcher-art-{index}"))
                .spawn(move || worker(mailbox))?;
        }

        Ok(cache)
    }

    pub fn set_budget(&mut self, bytes: usize) {
        if self.budget == bytes {
            return;
        }

        self.budget = bytes;
        while self.bytes > self.budget && self.evict_oldest() {}
    }

    fn evict_oldest(&mut self) -> bool {
        let Some(oldest) = self
            .resident
            .iter()
            .min_by_key(|(_, entry)| entry.used_at)
            .map(|(key, _)| key.clone())
        else {
            return false;
        };

        let old = self
            .resident
            .remove(&oldest)
            .expect("selected resident entry exists");

        self.bytes -= old.artwork.byte_len();
        true
    }

    pub fn set_generation(&mut self, generation: u64) {
        if generation == self.generation {
            return;
        }

        self.generation = generation;
        self.pending.clear();
        let (mailbox, wake) = &*self.mailbox;
        let mut mailbox = lock(mailbox);
        mailbox.generation = generation;
        mailbox.requests.clear();
        mailbox.completed.clear();
        wake.notify_all();
    }

    /// Returns the key even when the queue is full; callers retry next tick.
    pub fn request(
        &mut self,
        generation: u64,
        entry: &LibraryEntry,
        release: Option<Region>,
        max_dimension: u32,
    ) -> ArtworkKey {
        let key = ArtworkKey::new(entry, release, max_dimension);

        if generation != self.generation {
            return key;
        }

        if let Some(cached) = self.resident.get_mut(&key) {
            // Count a visible request as a use, since the renderer may not call `get`.
            self.tick = self.tick.wrapping_add(1);
            cached.used_at = self.tick;
            return key;
        }

        if self.pending.contains(&key)
            || self
                .failed
                .get(&key)
                .is_some_and(|failed| failed.elapsed() < Duration::from_secs(60))
        {
            return key;
        }

        let (mailbox, wake) = &*self.mailbox;
        let mut mailbox = lock(mailbox);

        if mailbox.requests.len() < REQUEST_CAPACITY {
            mailbox.requests.push_back(Request {
                generation,
                key: key.clone(),
            });

            self.pending.insert(key.clone());
            wake.notify_one();
        }

        key
    }

    pub fn poll(&mut self) -> Vec<ArtworkCompletion> {
        let results: Vec<_> = {
            let (mailbox, wake) = &*self.mailbox;
            let mut mailbox = lock(mailbox);

            if mailbox.completed.is_empty() {
                return Vec::new();
            }

            wake.notify_all();
            mailbox.completed.drain(..).collect()
        };

        results
            .into_iter()
            .filter_map(|completed| {
                if completed.generation != self.generation {
                    return None;
                }

                self.pending.remove(&completed.key);

                let result = completed.result.and_then(|artwork| {
                    for warning in &artwork.warnings {
                        tracing::warn!(%warning, key=?completed.key, "Artwork face unavailable");
                    }

                    self.insert(completed.key.clone(), artwork)
                });

                if result.is_err() {
                    if self.failed.len() >= REQUEST_CAPACITY * 8
                        && let Some(oldest) = self
                            .failed
                            .iter()
                            .min_by_key(|(_, failed)| **failed)
                            .map(|(key, _)| key.clone())
                    {
                        self.failed.remove(&oldest);
                    }

                    self.failed.insert(completed.key.clone(), Instant::now());
                }

                Some(ArtworkCompletion {
                    key: completed.key,
                    result,
                })
            })
            .collect()
    }

    fn insert(&mut self, key: ArtworkKey, artwork: DecodedArtwork) -> Result<(), String> {
        let size = artwork.byte_len();

        if size > self.budget {
            return Err("Artwork bundle exceeds CPU cache budget".into());
        }

        if let Some(old) = self.resident.remove(&key) {
            self.bytes -= old.artwork.byte_len();
        }

        while self.bytes.saturating_add(size) > self.budget {
            if !self.evict_oldest() {
                break;
            }
        }

        // Cap the record count too, since empty records cost no bytes.
        while self.resident.len() >= 128 {
            if !self.evict_oldest() {
                break;
            }
        }

        self.tick = self.tick.wrapping_add(1);
        self.bytes += size;

        self.resident.insert(
            key,
            Cached {
                artwork: Arc::new(artwork),
                used_at: self.tick,
            },
        );

        Ok(())
    }

    /// Clones held outside the cache don't count against its budget.
    pub fn get(&mut self, key: &ArtworkKey) -> Option<&Arc<DecodedArtwork>> {
        self.tick = self.tick.wrapping_add(1);
        let cached = self.resident.get_mut(key)?;
        cached.used_at = self.tick;
        Some(&cached.artwork)
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

impl Drop for ArtworkCache {
    fn drop(&mut self) {
        let (mailbox, wake) = &*self.mailbox;
        let mut mailbox = lock(mailbox);
        mailbox.shutdown = true;
        mailbox.requests.clear();
        mailbox.completed.clear();
        wake.notify_all();
    }
}

fn worker(shared: Arc<(Mutex<Mailbox>, Condvar)>) {
    let (mutex, wake) = &*shared;

    loop {
        let request = {
            let mut mailbox = lock(mutex);

            while mailbox.requests.is_empty() && !mailbox.shutdown {
                mailbox = wake
                    .wait(mailbox)
                    .unwrap_or_else(|error| error.into_inner());
            }

            if mailbox.shutdown {
                return;
            }

            mailbox.active += 1;

            mailbox
                .requests
                .pop_front()
                .expect("nonempty request queue")
        };

        let cancelled = || {
            let mailbox = lock(mutex);
            mailbox.shutdown || mailbox.generation != request.generation
        };

        let result = fetch_decode(&request.key, cancelled);
        let mut mailbox = lock(mutex);
        mailbox.active -= 1;

        // Bound decoded artwork waiting for upload, even while minimized.
        while mailbox.completed.len() >= COMPLETION_CAPACITY
            && !mailbox.shutdown
            && mailbox.generation == request.generation
        {
            mailbox = wake
                .wait(mailbox)
                .unwrap_or_else(|error| error.into_inner());
        }

        if mailbox.shutdown {
            return;
        }

        if mailbox.generation != request.generation {
            mailbox.stale_discarded += 1;
            continue;
        }

        mailbox.completed.push_back(Completed {
            generation: request.generation,
            key: request.key,
            result,
        });
    }
}

fn fetch_decode(key: &ArtworkKey, cancelled: impl Fn() -> bool) -> Result<DecodedArtwork, String> {
    let game = core_model::GameId::new(key.system, key.sha1.clone());
    let bundle = artwork::layle::fetch_bundle_strict(&game, key.release, &cancelled)?;
    let mut warnings = Vec::new();

    let mut decode_face = |name: &str, bytes: Option<Vec<u8>>| {
        if cancelled() {
            return None;
        }

        bytes.and_then(|bytes| match decode(&bytes, key.tier) {
            Ok(image) => Some(image),
            Err(error) => {
                warnings.push(format!("{name}: {error}"));
                None
            }
        })
    };

    let front = decode_face("front", bundle.front);
    let back = decode_face("back", bundle.back);
    let spine = decode_face("spine", bundle.spine);
    let label = decode_face("label", bundle.label_front.or(bundle.label));
    let label_top = decode_face("label top", bundle.label_top);

    if cancelled() {
        return Err("Artwork request cancelled".into());
    }

    let accent = front.as_ref().map(sample_cover_accent).unwrap_or_default();

    Ok(DecodedArtwork {
        title: bundle.title,
        accent,
        front,
        back,
        spine,
        label,
        label_top,
        warnings,
    })
}

fn decode(bytes: &[u8], tier: u32) -> Result<DecodedImage, String> {
    let dimensions = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .into_dimensions()
        .map_err(|error| error.to_string())?;

    if dimensions.0 > 8192
        || dimensions.1 > 8192
        || u64::from(dimensions.0) * u64::from(dimensions.1) * 4 > MAX_DECODE_BYTES
    {
        return Err("Image exceeds the decoded RGBA byte limit".into());
    }

    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| error.to_string())?;
    let tier = tier.clamp(64, MAX_TIER);
    let rgba = image.into_rgba8();

    let mut image = DecodedImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        mip_levels: Vec::new(),
        border_average: [1.0; 3],
    };

    build_mips(&mut image);

    // Use the first mip that fits the tier, so downsampling stays in linear light.
    if image.width > tier || image.height > tier {
        let index = image
            .mip_levels
            .iter()
            .position(|mip| mip.width <= tier && mip.height <= tier)
            .expect("mip chain always reaches one pixel");

        let level = image
            .mip_levels
            .drain(..=index)
            .next_back()
            .expect("selected mip exists");

        image.width = level.width;
        image.height = level.height;
        image.rgba = level.rgba;
    }

    image.border_average = sample_border_average(&image);
    Ok(image)
}

fn sample_cover_accent(image: &DecodedImage) -> CoverAccent {
    const HUE_BINS: usize = 36;

    #[derive(Clone, Copy, Default)]
    struct Bin {
        weight: f32,
        linear: [f32; 3],
    }

    let (width, height) = (image.width as usize, image.height as usize);

    if width == 0 || height == 0 || image.rgba.len() != width * height * 4 {
        return CoverAccent::default();
    }

    let mut bins = [Bin::default(); HUE_BINS];
    let (nx, ny) = (width.min(128), height.min(128));

    for sy in 0..ny {
        let y = (2 * sy + 1) * height / (2 * ny);

        for sx in 0..nx {
            let x = (2 * sx + 1) * width / (2 * nx);
            let pixel = &image.rgba[(y * width + x) * 4..][..4];

            if pixel[3] == 0 {
                continue;
            }

            let linear = std::array::from_fn(|i| srgb8_to_linear(pixel[i]));
            let [lightness, a, b] = linear_to_oklab(linear);
            let chroma = a.hypot(b);

            if lightness < 0.22 || (lightness > 0.94 && chroma < 0.08) || chroma < 0.045 {
                continue;
            }

            let hue = b.atan2(a).rem_euclid(std::f32::consts::TAU);
            let index =
                ((hue / std::f32::consts::TAU * HUE_BINS as f32) as usize).min(HUE_BINS - 1);
            let weight = f32::from(pixel[3]) / 255.0 * (0.5 + 0.5 * (chroma / 0.15).min(1.0));
            let bin = &mut bins[index];
            bin.weight += weight;

            for (sum, channel) in bin.linear.iter_mut().zip(linear) {
                *sum += channel * weight;
            }
        }
    }

    // Sum neighboring bins so a hue split across a bin boundary still wins. Hue wraps at red.
    let mut best = Bin::default();

    for index in 0..HUE_BINS {
        let mut group = Bin::default();

        for i in [
            (index + HUE_BINS - 1) % HUE_BINS,
            index,
            (index + 1) % HUE_BINS,
        ] {
            group.weight += bins[i].weight;

            for (sum, channel) in group.linear.iter_mut().zip(bins[i].linear) {
                *sum += channel;
            }
        }

        if group.weight > best.weight {
            best = group;
        }
    }

    if best.weight == 0.0 {
        return CoverAccent::default();
    }

    let linear = best.linear.map(|sum| (sum / best.weight).clamp(0.0, 1.0));

    CoverAccent {
        srgb: linear.map(linear_to_srgb),
        linear,
    }
}

fn linear_to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    // Public-domain transform by Bjorn Ottosson, 2021 matrix revision:
    // https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();

    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn sample_border_average(image: &DecodedImage) -> [f32; 3] {
    #[derive(Default)]
    struct Mean {
        rgb: [f64; 3],
        alpha: f64,
    }

    impl Mean {
        fn add(&mut self, pixel: &[u8]) {
            let alpha = f64::from(pixel[3]) / 255.0;
            self.alpha += alpha;

            for (value, channel) in self.rgb.iter_mut().zip(pixel) {
                *value += f64::from(srgb8_to_linear(*channel)) * alpha;
            }
        }

        fn finish(self) -> [f32; 3] {
            if self.alpha > 0.0 {
                self.rgb.map(|value| (value / self.alpha) as f32)
            } else {
                [1.0; 3]
            }
        }
    }

    let (width, height) = (image.width as usize, image.height as usize);

    if width == 0 || height == 0 || image.rgba.len() != width * height * 4 {
        return [1.0; 3];
    }

    let x_band = width.div_ceil(50).clamp(1, 16);
    let y_band = height.div_ceil(50).clamp(1, 16);
    let mut average = Mean::default();

    let mut sample = |x: usize, y: usize| {
        average.add(&image.rgba[(y * width + x) * 4..][..4]);
    };

    for y in 0..height {
        if y < y_band || y >= height - y_band {
            for x in 0..width {
                sample(x, y);
            }
        } else {
            for x in 0..x_band {
                sample(x, y);
            }

            for x in (width - x_band).max(x_band)..width {
                sample(x, y);
            }
        }
    }

    average.finish()
}

fn build_mips(image: &mut DecodedImage) {
    static TO_SRGB: LazyLock<[u8; 4097]> = LazyLock::new(|| {
        std::array::from_fn(|value| (linear_to_srgb(value as f32 / 4096.0) * 255.0).round() as u8)
    });

    let (mut width, mut height) = (image.width, image.height);

    while width > 1 || height > 1 {
        let source = image
            .mip_levels
            .last()
            .map(|mip| mip.rgba.as_slice())
            .unwrap_or(&image.rgba);

        let (next_width, next_height) = ((width / 2).max(1), (height / 2).max(1));
        let mut rgba = vec![0; next_width as usize * next_height as usize * 4];

        for y in 0..next_height {
            for x in 0..next_width {
                let (x0, x1) = (x * width / next_width, (x + 1) * width / next_width);
                let (y0, y1) = (y * height / next_height, (y + 1) * height / next_height);
                let mut rgb = [0.0; 3];
                let mut alpha = 0.0;

                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let pixel = &source[((sy * width + sx) * 4) as usize..][..4];
                        let a = pixel[3] as f32 / 255.0;
                        alpha += a;

                        for channel in 0..3 {
                            rgb[channel] += srgb8_to_linear(pixel[channel]) * a;
                        }
                    }
                }

                let output = &mut rgba[((y * next_width + x) * 4) as usize..][..4];

                if alpha > 0.0 {
                    for channel in 0..3 {
                        output[channel] = TO_SRGB
                            [(rgb[channel] / alpha * 4096.0).round().clamp(0.0, 4096.0) as usize];
                    }
                }

                output[3] = (alpha * 255.0 / ((x1 - x0) * (y1 - y0)) as f32).round() as u8;
            }
        }

        image.mip_levels.push(MipLevel {
            width: next_width,
            height: next_height,
            rgba,
        });

        (width, height) = (next_width, next_height);
    }
}
