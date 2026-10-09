//! BNR1 layout tests (offsets as read by the IPL / Swiss `BNR` struct).

use gc_bnr::{build_bnr1, parse_ppm, rgb5a3_opaque, BannerText, Rgb, BNR1_LEN, HEIGHT, WIDTH};

fn gradient() -> Rgb {
    let pixels = (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| [(x * 2) as u8, (y * 8) as u8, 0x40]))
        .collect();
    Rgb { width: WIDTH, height: HEIGHT, pixels }
}

fn text() -> BannerText {
    BannerText {
        game_name: "Yarn Cat".into(),
        company: "gc-rust".into(),
        full_game_name: "Yarn Cat - a kitten in your TV".into(),
        full_company: "gc-rust".into(),
        description: "line one\nline two".into(),
    }
}

#[test]
fn layout_and_text_offsets() {
    let bnr = build_bnr1(&gradient(), &text()).unwrap();
    assert_eq!(bnr.len(), BNR1_LEN);
    assert_eq!(&bnr[0..4], b"BNR1");
    assert!(bnr[4..0x20].iter().all(|&b| b == 0));
    assert_eq!(&bnr[0x1820..0x1828], b"Yarn Cat");
    assert_eq!(bnr[0x1828], 0);
    assert_eq!(&bnr[0x1840..0x1847], b"gc-rust");
    assert_eq!(&bnr[0x1860..0x1866], b"Yarn C");
    assert_eq!(&bnr[0x18a0..0x18a7], b"gc-rust");
    assert_eq!(&bnr[0x18e0..0x18f1], b"line one\nline two");
}

#[test]
fn pixels_are_tiled_4x4() {
    let img = gradient();
    let bnr = build_bnr1(&img, &text()).unwrap();
    let texel = |i: usize| u16::from_be_bytes([bnr[0x20 + 2 * i], bnr[0x20 + 2 * i + 1]]);
    // first tile row: texel 4 is (0,1), texel 16 starts the second tile (4,0)
    assert_eq!(texel(0), rgb5a3_opaque(img.pixels[0]));
    assert_eq!(texel(4), rgb5a3_opaque(img.pixels[WIDTH]));
    assert_eq!(texel(16), rgb5a3_opaque(img.pixels[4]));
    // second row of tiles starts after 24 tiles * 16 texels at (0,4)
    assert_eq!(texel(24 * 16), rgb5a3_opaque(img.pixels[4 * WIDTH]));
    // opaque bit always set
    assert!((0..WIDTH * HEIGHT).all(|i| texel(i) & 0x8000 != 0));
}

#[test]
fn rejects_bad_input() {
    let small = Rgb { width: 32, height: 32, pixels: vec![[0; 3]; 32 * 32] };
    assert!(build_bnr1(&small, &text()).is_err());
    let mut long = text();
    long.game_name = "x".repeat(0x20);
    assert!(build_bnr1(&gradient(), &long).is_err());
    let mut fancy = text();
    fancy.description = "caf\u{e9}".into();
    assert!(build_bnr1(&gradient(), &fancy).is_err());
}

#[test]
fn parses_ppm_with_comment() {
    let mut ppm = b"P6\n# made by a test\n2 1\n255\n".to_vec();
    ppm.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    let img = parse_ppm(&ppm).unwrap();
    assert_eq!((img.width, img.height), (2, 1));
    assert_eq!(img.pixels, vec![[1, 2, 3], [4, 5, 6]]);
    assert!(parse_ppm(b"P3\n1 1\n255\n0 0 0").is_err());
}

#[test]
fn example_banner_asset_builds() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/23-yarn-cat/banner.ppm");
    let img = parse_ppm(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(build_bnr1(&img, &text()).unwrap().len(), BNR1_LEN);
}
