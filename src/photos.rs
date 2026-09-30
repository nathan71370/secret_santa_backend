//! Photos de cadeaux : vérifiées sur leurs octets (jamais l'extension),
//! orientées, réduites, puis réencodées en JPEG — ce qui efface les
//! métadonnées EXIF, géolocalisation comprise.

use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder,
    imageops::FilterType, metadata::Orientation,
};
use std::{io::Cursor, path::Path};

pub const MAX_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_SIDE: u32 = 1600;
const MAX_DECODE_SIDE: u32 = 8_000;
const MAX_DECODE_PIXELS: u64 = 40_000_000;
const MAX_DECODE_ALLOC: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PhotoError {
    #[error("Photo trop lourde (5 Mo maximum).")]
    TooLarge,
    #[error("Format non pris en charge (JPEG, PNG ou WebP).")]
    UnsupportedFormat,
    #[error("Image illisible.")]
    Unreadable,
}

pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>, PhotoError> {
    if bytes.len() > MAX_BYTES {
        return Err(PhotoError::TooLarge);
    }
    let format = image::guess_format(bytes).map_err(|_| PhotoError::UnsupportedFormat)?;
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
    ) {
        return Err(PhotoError::UnsupportedFormat);
    }
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|_| PhotoError::Unreadable)?;
    // Garde-fou contre les images « bombes » : petites en octets, énormes une
    // fois décodées. La surface déclarée est vérifiée avant tout décodage.
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_DECODE_PIXELS {
        return Err(PhotoError::TooLarge);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    decoder
        .set_limits(limits)
        .map_err(|_| PhotoError::Unreadable)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| PhotoError::Unreadable)?;
    img.apply_orientation(orientation);
    if img.width() > MAX_SIDE || img.height() > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, FilterType::Lanczos3);
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&img.to_rgb8())
        .map_err(|_| PhotoError::Unreadable)?;
    Ok(out)
}

/// `<uuid v4>.jpg` uniquement : ferme la porte aux `../` sur la route de lecture.
pub fn is_valid_name(name: &str) -> bool {
    name.strip_suffix(".jpg")
        .is_some_and(|stem| stem.len() == 36 && uuid::Uuid::parse_str(stem).is_ok())
}

pub async fn store(dir: &Path, jpeg: &[u8]) -> std::io::Result<String> {
    tokio::fs::create_dir_all(dir).await?;
    let name = format!("{}.jpg", uuid::Uuid::new_v4());
    tokio::fs::write(dir.join(&name), jpeg).await?;
    Ok(name)
}

pub async fn remove(dir: &Path, name: &str) {
    if is_valid_name(name)
        && let Err(e) = tokio::fs::remove_file(dir.join(name)).await
    {
        tracing::warn!(error = %e, "photo non supprimée");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbImage};
    use std::io::Cursor;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])))
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// CRC des blocs PNG, pour garder un en-tête IHDR valide après retouche.
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn reduit_et_reencode_en_jpeg() {
        let jpeg = normalize(&png(3200, 1000)).unwrap();
        assert_eq!(image::guess_format(&jpeg).unwrap(), ImageFormat::Jpeg);
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (1600, 500));
    }

    #[test]
    fn petite_image_gardee_a_sa_taille() {
        let img = image::load_from_memory(&normalize(&png(400, 300)).unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (400, 300));
    }

    #[test]
    fn refuse_ce_qui_n_est_pas_une_image_prise_en_charge() {
        assert_eq!(normalize(b"bonjour"), Err(PhotoError::UnsupportedFormat));
        assert_eq!(
            normalize(b"GIF89a\x01\x00\x01\x00\x00\x00\x00"),
            Err(PhotoError::UnsupportedFormat)
        );
    }

    #[test]
    fn refuse_plus_de_5_mo() {
        assert_eq!(
            normalize(&vec![0u8; MAX_BYTES + 1]),
            Err(PhotoError::TooLarge)
        );
    }

    #[test]
    fn nom_multi_octets_sans_panique() {
        let nom = format!("é{}.jpg", "a".repeat(34));
        assert_eq!(nom.len(), 40);
        assert!(!is_valid_name(&nom));
        let nom = format!("{}é{}.jpg", "a".repeat(35), "a");
        assert!(!is_valid_name(&nom));
    }

    #[test]
    fn refuse_les_dimensions_declarees_demesurees() {
        let mut bytes = png(8, 8);
        bytes[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&20_000u32.to_be_bytes());
        assert!(normalize(&bytes).is_err());
    }

    #[test]
    fn refuse_plus_de_40_megapixels_avant_decodage() {
        // 7000 × 7000 = 49 Mpx : chaque côté passe, la surface non.
        let mut bytes = png(8, 8);
        bytes[16..20].copy_from_slice(&7_000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&7_000u32.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(normalize(&bytes), Err(PhotoError::TooLarge));
    }

    #[test]
    fn noms_de_fichier() {
        assert!(is_valid_name("0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.jpg"));
        assert!(!is_valid_name("../santa.db"));
        assert!(!is_valid_name("0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.png"));
        assert!(!is_valid_name(""));
    }
}
