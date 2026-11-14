//! Original, editable starter designs. No third-party template artwork.
use crate::{design::*, template::Template};

pub const PRESETS: [(&str, &str); 10] = [
    ("The Editorial", "Warm paper · classic strip"),
    ("After Hours", "Midnight blue · gold details"),
    ("Garden Party", "Sage green · wedding grid"),
    ("Pop Club", "Coral + lilac · playful shapes"),
    ("Golden Hour", "Peach gradient · double strip"),
    ("Made of Memories", "Tilted prints · scrapbook"),
    ("Electric Night", "Violet glow · party strip"),
    ("Birthday Wishes", "Confetti · bright & colorful"),
    ("Modern Love", "Blush + hearts · wedding strip"),
    ("Less Is More", "Black & cream · clean grid"),
];
pub const PALETTES: [(&str, [Color; 4]); 8] = [
    (
        "Warm neutrals",
        [
            [247, 239, 225, 255],
            [218, 181, 151, 255],
            [177, 104, 79, 255],
            [49, 42, 36, 255],
        ],
    ),
    (
        "Garden",
        [
            [236, 240, 224, 255],
            [155, 177, 142, 255],
            [75, 104, 81, 255],
            [35, 55, 43, 255],
        ],
    ),
    (
        "Midnight",
        [
            [17, 29, 51, 255],
            [41, 62, 91, 255],
            [198, 166, 103, 255],
            [248, 238, 213, 255],
        ],
    ),
    (
        "Candy",
        [
            [255, 229, 220, 255],
            [255, 126, 119, 255],
            [186, 166, 231, 255],
            [62, 41, 93, 255],
        ],
    ),
    (
        "Ocean",
        [
            [230, 247, 243, 255],
            [121, 187, 191, 255],
            [35, 112, 139, 255],
            [17, 54, 77, 255],
        ],
    ),
    (
        "Sunset",
        [
            [255, 222, 177, 255],
            [244, 156, 129, 255],
            [206, 94, 118, 255],
            [97, 47, 82, 255],
        ],
    ),
    (
        "Neon",
        [
            [27, 13, 56, 255],
            [97, 46, 137, 255],
            [223, 98, 203, 255],
            [102, 240, 224, 255],
        ],
    ),
    (
        "Mono",
        [
            [248, 246, 238, 255],
            [202, 199, 190, 255],
            [110, 107, 100, 255],
            [29, 28, 26, 255],
        ],
    ),
];
fn title(
    t: &mut Template,
    text: &str,
    x: f64,
    y: f64,
    w: f64,
    size: f64,
    color: Color,
    family: FontFamily,
) {
    let mut l = Layer::text(text, Frame::new(x, y, w, size * 2.6));
    l.name = text.into();
    if let LayerKind::Text {
        family: f,
        size: s,
        color: c,
        ..
    } = &mut l.kind
    {
        *f = family;
        *s = size;
        *c = color;
    }
    t.layers.push(l);
}
fn dot(t: &mut Template, x: f64, y: f64, size: f64, color: Color) {
    t.layers.push(Layer::shape(
        ShapeKind::Ellipse,
        Frame::new(x, y, size, size),
        color,
    ));
}
pub fn build(index: usize) -> Template {
    let mut t = if matches!(index, 2 | 9) {
        Template::grid()
    } else {
        Template::builtin(matches!(index, 3 | 4 | 5 | 7))
    };
    t.name = PRESETS[index].0.into();
    let palette = PALETTES[match index {
        1 => 2,
        2 => 1,
        3 | 7 => 3,
        4 => 5,
        6 => 6,
        9 => 7,
        _ => 0,
    }]
    .1;
    t.background.color = palette[0];
    t.background.secondary = palette[1];
    match index {
        0 | 1 | 6 | 8 => {
            t.layers = (0..3)
                .map(|i| Layer::photo(Frame::new(42.0, 220.0 + i as f64 * 420.0, 516.0, 387.0), i))
                .collect();
            if index == 6 {
                t.background.style = BackgroundStyle::Gradient;
                t.background.angle = 60.0;
            }
            title(
                &mut t,
                match index {
                    1 => "AFTER HOURS",
                    6 => "GOOD ENERGY",
                    8 => "Modern Love",
                    _ => "The good days",
                },
                30.0,
                65.0,
                540.0,
                if index == 8 { 64.0 } else { 52.0 },
                palette[3],
                if index == 6 {
                    FontFamily::Mono
                } else {
                    FontFamily::Serif
                },
            );
            title(
                &mut t,
                match index {
                    1 => "A NIGHT TO REMEMBER",
                    6 => "DANCE. LAUGH. REPEAT.",
                    8 => "TOGETHER IS A BEAUTIFUL PLACE",
                    _ => "COLLECT LITTLE MOMENTS",
                },
                30.0,
                1570.0,
                540.0,
                23.0,
                palette[3],
                FontFamily::Sans,
            );
            title(
                &mut t,
                "YOUR EVENT  /  2026",
                30.0,
                1660.0,
                540.0,
                22.0,
                palette[3],
                FontFamily::Mono,
            );
            if index == 8 {
                for (x, y) in [(22.0, 1470.0), (500.0, 1500.0)] {
                    t.layers.push(Layer::shape(
                        ShapeKind::Heart,
                        Frame::new(x, y, 65.0, 65.0),
                        [197, 103, 108, 255],
                    ));
                }
            } else {
                let mut l = Layer::shape(
                    ShapeKind::Rectangle,
                    Frame::new(95.0, 1520.0, 410.0, 3.0),
                    palette[2],
                );
                l.name = "Fine rule".into();
                t.layers.push(l);
            }
        }
        2 | 9 => {
            t.width = 1800;
            t.height = 1200;
            t.layers.clear();
            if index == 2 {
                t.background.style = BackgroundStyle::Dots;
                t.background.spacing = 75.0;
                for (x, y) in [(30.0, 25.0), (1670.0, 1030.0)] {
                    dot(&mut t, x, y, 100.0, palette[1]);
                }
            }
            for i in 0..4 {
                t.layers.push(Layer::photo(
                    Frame::new(
                        90.0 + (i % 2) as f64 * 855.0,
                        235.0 + (i / 2) as f64 * 420.0,
                        765.0,
                        365.0,
                    ),
                    i,
                ));
            }
            title(
                &mut t,
                if index == 2 {
                    "Better together"
                } else {
                    "HERE & NOW"
                },
                180.0,
                40.0,
                1440.0,
                86.0,
                palette[3],
                if index == 2 {
                    FontFamily::Serif
                } else {
                    FontFamily::Sans
                },
            );
            title(
                &mut t,
                if index == 2 {
                    "THE WEDDING OF  /  YOUR NAMES"
                } else {
                    "YOUR PEOPLE. YOUR MOMENT. YOUR MEMORIES."
                },
                120.0,
                1095.0,
                1560.0,
                30.0,
                palette[3],
                FontFamily::Mono,
            );
        }
        4 => {
            t = Template::builtin(true);
            t.name = PRESETS[index].0.into();
            t.background.color = palette[0];
            t.background.secondary = palette[2];
            t.background.style = BackgroundStyle::Gradient;
            t.background.angle = 105.0;
            t.layers.clear();
            for col in 0..2 {
                for i in 0..3 {
                    t.layers.push(Layer::photo(
                        Frame::new(
                            45.0 + col as f64 * 600.0,
                            215.0 + i as f64 * 425.0,
                            510.0,
                            382.5,
                        ),
                        i,
                    ));
                }
                title(
                    &mut t,
                    "Golden hour",
                    col as f64 * 600.0 + 25.0,
                    65.0,
                    550.0,
                    60.0,
                    palette[3],
                    FontFamily::Serif,
                );
                title(
                    &mut t,
                    "SUN-KISSED MEMORIES",
                    col as f64 * 600.0 + 25.0,
                    1580.0,
                    550.0,
                    24.0,
                    palette[3],
                    FontFamily::Sans,
                );
            }
        }
        5 => {
            t.width = 1200;
            t.height = 1800;
            t.layers.clear();
            t.background.color = [239, 226, 204, 255];
            t.background.style = BackgroundStyle::Dots;
            t.background.secondary = [209, 191, 163, 255];
            t.background.spacing = 45.0;
            for (i, (x, y, rotation)) in [
                (100.0, 300.0, -7.0),
                (620.0, 650.0, 8.0),
                (110.0, 1050.0, -4.0),
            ]
            .into_iter()
            .enumerate()
            {
                let mut card = Layer::shape(
                    ShapeKind::Rectangle,
                    Frame::new(x, y, 510.0, 510.0),
                    [255, 252, 242, 255],
                );
                card.frame.rotation = rotation;
                t.layers.push(card);
                let mut photo =
                    Layer::photo(Frame::new(x + 25.0, y + 25.0, 450.0, 350.0), i as u32);
                photo.frame.rotation = rotation;
                t.layers.push(photo);
            }
            title(
                &mut t,
                "Made of memories",
                40.0,
                70.0,
                1120.0,
                90.0,
                palette[3],
                FontFamily::Serif,
            );
            title(
                &mut t,
                "KEEP THIS ONE FOREVER",
                100.0,
                1630.0,
                1000.0,
                36.0,
                palette[3],
                FontFamily::Mono,
            );
        }
        _ => {
            t.width = 1200;
            t.height = 1800;
            t.layers.clear();
            t.background.color = palette[0];
            for i in 0..22 {
                let x = (i * 173 % 1150) as f64;
                let y = (i * 397 % 1750) as f64;
                let mut l = Layer::shape(
                    if i % 3 == 0 {
                        ShapeKind::Star
                    } else {
                        ShapeKind::Ellipse
                    },
                    Frame::new(
                        x,
                        y,
                        35.0 + (i % 3) as f64 * 18.0,
                        35.0 + (i % 3) as f64 * 18.0,
                    ),
                    palette[1 + (i as usize % 2)],
                );
                l.frame.rotation = i as f64 * 17.0;
                t.layers.push(l);
            }
            for i in 0..4 {
                t.layers.push(Layer::photo(
                    Frame::new(
                        95.0 + (i % 2) as f64 * 540.0,
                        400.0 + (i / 2) as f64 * 530.0,
                        470.0,
                        460.0,
                    ),
                    i,
                ));
            }
            title(
                &mut t,
                if index == 7 {
                    "MAKE A WISH!"
                } else {
                    "POP CLUB"
                },
                60.0,
                100.0,
                1080.0,
                110.0,
                palette[3],
                FontFamily::Sans,
            );
            title(
                &mut t,
                if index == 7 {
                    "HAPPY BIRTHDAY, YOUR NAME"
                } else {
                    "A LITTLE LOUD. A LOT OF FUN."
                },
                70.0,
                260.0,
                1060.0,
                36.0,
                palette[3],
                FontFamily::Mono,
            );
            title(
                &mut t,
                "BEST NIGHT. BEST PEOPLE.",
                70.0,
                1570.0,
                1060.0,
                40.0,
                palette[3],
                FontFamily::Sans,
            );
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_original_preset_renders_and_saves_as_editable_sbl() {
        for i in 0..PRESETS.len() {
            let t = build(i);
            t.validate().unwrap();
            let before = t.render(&[], Some(240)).unwrap();
            let restored = Template::from_bytes(&t.to_bytes().unwrap()).unwrap();
            assert_eq!(t.layers.len(), restored.layers.len());
            assert_eq!(
                before.as_bytes(),
                restored.render(&[], Some(240)).unwrap().as_bytes(),
                "{}",
                t.name
            );
        }
    }
}
