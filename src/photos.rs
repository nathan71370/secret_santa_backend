//! Photos de cadeaux : vérifiées sur leurs octets (jamais l'extension),
//! orientées, réduites, puis réencodées en JPEG — ce qui efface les
//! métadonnées EXIF, géolocalisation comprise.

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, codecs::jpeg::JpegEncoder, imageops::FilterType, metadata::Orientation};
use std::{io::Cursor, path::Path};

pub const MAX_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_SIDE: u32 = 1600;

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
    if !matches!(format, ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP) {
        return Err(PhotoError::UnsupportedFormat);
    }
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
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
    name.len() == 40
        && name.ends_with(".jpg")
        && name[..36].chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

pub async fn store(dir: &Path, jpeg: &[u8]) -> std::io::Result<String> {
    tokio::fs::create_dir_all(dir).await?;
    let name = format!("{}.jpg", uuid::Uuid::new_v4());
    tokio::fs::write(dir.join(&name), jpeg).await?;
    Ok(name)
}

pub async fn remove(dir: &Path, name: &str) {
    if is_valid_name(name) {
        if let Err(e) = tokio::fs::remove_file(dir.join(name)).await {
            tracing::warn!(error = %e, "photo non supprimée");
        }
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
        assert_eq!(normalize(b"GIF89a\x01\x00\x01\x00\x00\x00\x00"), Err(PhotoError::UnsupportedFormat));
    }

    #[test]
    fn refuse_plus_de_5_mo() {
        assert_eq!(normalize(&vec![0u8; MAX_BYTES + 1]), Err(PhotoError::TooLarge));
    }

    #[test]
    fn noms_de_fichier() {
        assert!(is_valid_name("0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.jpg"));
        assert!(!is_valid_name("../santa.db"));
        assert!(!is_valid_name("0f8c2c1e-4b6a-4c43-9a55-2f1d7a3b9e10.png"));
        assert!(!is_valid_name(""));
    }
}
