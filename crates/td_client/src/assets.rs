//! Packs and the few textures the client makes itself.
//!
//! Native: `--pack-dir`, else `packs/base` next to the working directory or
//! the executable. Web: the base pack zip embedded at build time
//! (`TD_EMBED_PACK`), since there is no file system.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use td_content::{Library, PackFiles};

use crate::Options;

pub fn plugin(app: &mut App) {
    // Loaded while building the app, so the menu has it on its first frame.
    let options = app.world().resource::<Options>().clone();
    match library(&options) {
        Ok(lib) => {
            app.insert_resource(Packs {
                lib: Arc::new(lib),
                images: HashMap::new(),
                circle: Handle::default(),
                dot: Handle::default(),
            });
        }
        Err(e) => {
            error!("{e}");
            app.insert_resource(LoadError(e));
        }
    }
    app.add_systems(PreStartup, textures);
}

#[derive(Resource, Clone)]
pub struct Packs {
    pub lib: Arc<Library>,
    /// Sprite path → texture, for files found under `assets/` in a pack.
    pub images: HashMap<String, Handle<Image>>,
    /// White disc, tinted per unit.
    pub circle: Handle<Image>,
    /// Soft white dot for particles.
    pub dot: Handle<Image>,
}

impl Packs {
    pub fn sprite(&self, path: &Option<String>) -> Option<Handle<Image>> {
        path.as_ref().and_then(|p| self.images.get(p).cloned())
    }
}

/// Why the packs failed to load, shown by the menu.
#[derive(Resource)]
pub struct LoadError(pub String);

/// With the `embed-pack` feature, `TD_EMBED_PACK` names a base pack zip to
/// bake into the binary (always used for the web build).
#[cfg(feature = "embed-pack")]
const EMBEDDED: Option<&[u8]> = Some(include_bytes!(env!("TD_EMBED_PACK")));
#[cfg(not(feature = "embed-pack"))]
const EMBEDDED: Option<&[u8]> = None;

fn base_files(options: &Options) -> Result<PackFiles, String> {
    if let Some(bytes) = EMBEDDED {
        return PackFiles::from_zip(bytes);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = &options.pack_dir {
            return PackFiles::open(dir);
        }
        let mut candidates = vec![std::path::PathBuf::from("packs/base")];
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            candidates.push(dir.join("packs/base"));
            candidates.push(dir.join("base.zip"));
            candidates.push(dir.join("../../packs/base"));
        }
        for c in &candidates {
            if c.join("info.toml").exists() || c.is_file() {
                return PackFiles::open(c);
            }
        }
        Err("base pack not found: run from the repository root or pass --pack-dir".into())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = options;
        Err("no embedded base pack".into())
    }
}

fn library(options: &Options) -> Result<Library, String> {
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut packs = vec![base_files(options)?];
    #[cfg(not(target_arch = "wasm32"))]
    for p in &options.packs {
        packs.push(PackFiles::open(p)?);
    }
    let lib = Library::load(packs)?;
    if lib.module_bytes("waves").is_none() {
        return Err("the base pack has no modules: run `cargo xtask modules`".into());
    }
    Ok(lib)
}

fn disc(size: u32, softness: f32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / c;
            let a = ((1.0 - d) / softness).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn textures(packs: Option<ResMut<Packs>>, mut images: ResMut<Assets<Image>>) {
    let Some(mut packs) = packs else { return };
    packs.circle = images.add(disc(64, 0.06));
    packs.dot = images.add(disc(32, 1.0));
    let lib = packs.lib.clone();
    for files in &lib.files {
        for (path, bytes) in &files.files {
            let Some(rel) = path.strip_prefix("assets/") else {
                continue;
            };
            if !rel.ends_with(".png") {
                continue;
            }
            match Image::from_buffer(
                bytes,
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::nearest(),
                RenderAssetUsages::RENDER_WORLD,
            ) {
                Ok(img) => {
                    let handle = images.add(img);
                    packs.images.insert(rel.to_string(), handle);
                }
                Err(e) => warn!("{path}: {e}"),
            }
        }
    }
}
