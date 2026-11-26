//! Marine animals for the 3D viewport: the species recorded at a place
//! (OBIS occurrence checklists) drawn with models built from each family's
//! body shape (proportions, fins, tail type, locomotion) and the species'
//! known colouring where catalogued (else the family's typical colouring),
//! living around the camera in their habitat and depth range: reef fish over
//! reefs, pelagic schools in open water, rays and flatfish on the bottom,
//! dolphins near the surface. Animals are visual: individual tracks are not
//! data, only which species occur here is.
use crate::math::Vec3;
use crate::render_scene::DrawVertex;

// ------------------------------------------------------------ catalogue

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Fish,
    Shark,
    Ray,
    Turtle,
    Cetacean,
    Eel,
    /// Flatfish: lies on one side on the bottom.
    Flat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tail {
    Forked,
    Lunate,
    Rounded,
    Truncate,
    /// Sharks: long upper lobe.
    Heterocercal,
    /// Whales and dolphins: horizontal flukes.
    Fluke,
    /// Eels: the fins run round a pointed tail.
    Pointed,
}
/// Where in the water an animal lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Habit {
    /// Over and around reef, near the bottom.
    Reef,
    /// Open water.
    Pelagic,
    /// Near the bottom.
    Demersal,
    /// On the bottom.
    Benthic,
    /// Near the surface (dolphins, flying fish, whale sharks feeding).
    Surface,
}
/// Body plan: proportions as fractions of total length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub shape: Shape,
    /// Typical adult length (m); disc width for rays.
    pub length: f64,
    pub depth: f64,
    pub width: f64,
    /// Where the body is deepest (fraction from the snout).
    pub peak: f64,
    /// 0 blunt head, 1 pointed.
    pub snout: f64,
    /// Depth of the tail stem relative to the deepest point.
    pub peduncle: f64,
    pub tail: Tail,
    pub tail_span: f64,
    /// Fins: [start, end, height] along the body (fractions).
    pub dorsal: [f64; 3],
    pub dorsal2: [f64; 3],
    pub anal: [f64; 3],
    pub pectoral: f64,
    /// Bill or rostrum length (billfish, needlefish).
    pub bill: f64,
    pub habit: Habit,
    /// Depth range (m below the surface).
    pub depths: [f64; 2],
    /// Usual group size (min, max).
    pub group: [u16; 2],
    /// Cruising speed (body lengths per second).
    pub speed: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coat {
    Plain,
    /// Vertical bars over the whole side.
    Bars(u8),
    /// Bars on the upper side only.
    Saddles(u8),
    /// Stripes along the body.
    Stripes(u8),
    Spots(u8),
    Mottled,
    /// Front in the body colour, rear in the accent.
    Fade,
}
/// Colouring (linear RGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub back: [f32; 3],
    pub belly: [f32; 3],
    pub fin: [f32; 3],
    pub tail: [f32; 3],
    pub accent: [f32; 3],
    pub coat: Coat,
    pub face: Option<[f32; 3]>,
    /// A dark band through the eye.
    pub eye_bar: bool,
    /// Silvery sheen (0-1).
    pub metallic: f32,
}

fn srgb(c: u32) -> [f32; 3] {
    let f = |v: u32| ((v & 255) as f32 / 255.0).powf(2.2);
    [f(c >> 16), f(c >> 8), f(c)]
}
fn look(back: u32, belly: u32, fin: u32, coat: Coat, accent: u32) -> Look {
    Look {
        back: srgb(back),
        belly: srgb(belly),
        fin: srgb(fin),
        tail: srgb(fin),
        accent: srgb(accent),
        coat,
        face: None,
        eye_bar: false,
        metallic: 0.05,
    }
}
impl Look {
    fn tail(mut self, c: u32) -> Self {
        self.tail = srgb(c);
        self
    }
    fn face(mut self, c: u32) -> Self {
        self.face = Some(srgb(c));
        self
    }
    fn eyebar(mut self) -> Self {
        self.eye_bar = true;
        self
    }
    fn shiny(mut self, m: f32) -> Self {
        self.metallic = m;
        self
    }
}

/// A bony-fish plan with common defaults; families adjust it.
#[allow(clippy::too_many_arguments)]
fn fish(
    length: f64,
    depth: f64,
    width: f64,
    tail: Tail,
    habit: Habit,
    depths: [f64; 2],
    group: [u16; 2],
    speed: f64,
) -> Plan {
    Plan {
        shape: Shape::Fish,
        length,
        depth,
        width,
        peak: 0.38,
        snout: 0.45,
        peduncle: 0.3,
        tail,
        tail_span: (depth * 0.8).clamp(0.18, 0.4),
        dorsal: [0.25, 0.8, depth * 0.18],
        dorsal2: [0.0; 3],
        anal: [0.55, 0.8, depth * 0.15],
        pectoral: 0.14,
        bill: 0.0,
        habit,
        depths,
        group,
        speed,
    }
}
fn shark(length: f64, habit: Habit, depths: [f64; 2], group: [u16; 2]) -> Plan {
    Plan {
        shape: Shape::Shark,
        length,
        depth: 0.17,
        width: 0.15,
        peak: 0.35,
        snout: 0.75,
        peduncle: 0.18,
        tail: Tail::Heterocercal,
        tail_span: 0.3,
        dorsal: [0.32, 0.45, 0.13],
        dorsal2: [0.7, 0.75, 0.04],
        anal: [0.7, 0.75, 0.035],
        pectoral: 0.2,
        bill: 0.0,
        habit,
        depths,
        group,
        speed: 0.5,
    }
}
fn ray(width: f64, habit: Habit, depths: [f64; 2], group: [u16; 2], speed: f64) -> Plan {
    Plan {
        shape: Shape::Ray,
        length: width,
        depth: 0.08,
        width: 1.0,
        peak: 0.4,
        snout: 0.3,
        peduncle: 0.1,
        tail: Tail::Pointed,
        tail_span: 0.0,
        dorsal: [0.0; 3],
        dorsal2: [0.0; 3],
        anal: [0.0; 3],
        pectoral: 0.0,
        bill: 0.0,
        habit,
        depths,
        group,
        speed,
    }
}
fn cetacean(length: f64, snout: f64, dorsal: f64, group: [u16; 2], speed: f64) -> Plan {
    Plan {
        shape: Shape::Cetacean,
        length,
        depth: 0.18,
        width: 0.17,
        peak: 0.38,
        snout,
        peduncle: 0.1,
        tail: Tail::Fluke,
        tail_span: 0.26,
        dorsal: [0.42, 0.55, dorsal],
        dorsal2: [0.0; 3],
        anal: [0.0; 3],
        pectoral: 0.15,
        bill: 0.0,
        habit: Habit::Surface,
        depths: [0.0, 200.0],
        group,
        speed,
    }
}
fn turtle(length: f64) -> Plan {
    Plan {
        shape: Shape::Turtle,
        length,
        depth: 0.25,
        width: 0.7,
        peak: 0.45,
        snout: 0.3,
        peduncle: 0.2,
        tail: Tail::Pointed,
        tail_span: 0.0,
        dorsal: [0.0; 3],
        dorsal2: [0.0; 3],
        anal: [0.0; 3],
        pectoral: 0.45,
        bill: 0.0,
        habit: Habit::Reef,
        depths: [0.0, 60.0],
        group: [1, 1],
        speed: 0.35,
    }
}

use Coat::*;
use Habit::*;
use Tail::*;

/// A family: common name, body plan and typical colouring.
pub fn family(name: &str) -> Option<(&'static str, Plan, Look)> {
    let reef = |l, d, w, t, g: [u16; 2]| fish(l, d, w, t, Reef, [0.5, 40.0], g, 0.9);
    Some(match name {
        "Acanthuridae" => (
            "surgeonfish",
            Plan {
                snout: 0.5,
                peduncle: 0.22,
                dorsal: [0.18, 0.82, 0.07],
                anal: [0.4, 0.8, 0.06],
                ..reef(0.25, 0.5, 0.12, Lunate, [1, 12])
            },
            look(0x52565c, 0x80858b, 0x45484d, Plain, 0x2a2c30),
        ),
        "Chaetodontidae" => (
            "butterflyfish",
            Plan {
                snout: 0.75,
                dorsal: [0.2, 0.86, 0.12],
                anal: [0.5, 0.86, 0.1],
                ..reef(0.15, 0.62, 0.09, Truncate, [1, 3])
            },
            look(0xe8e4d8, 0xf2efe6, 0xf0c419, Fade, 0xf3c623).eyebar(),
        ),
        "Pomacanthidae" => (
            "angelfish",
            Plan {
                dorsal: [0.25, 0.9, 0.1],
                anal: [0.5, 0.9, 0.1],
                ..reef(0.3, 0.6, 0.11, Rounded, [1, 2])
            },
            look(0x1f3a7a, 0x2d4f96, 0xf2b81c, Stripes(6), 0xf2c94c),
        ),
        "Pomacentridae" => (
            "damselfish",
            Plan {
                speed: 1.2,
                depths: [0.5, 20.0],
                ..reef(0.08, 0.5, 0.14, Forked, [6, 40])
            },
            look(0x4fa8c8, 0x9fe0de, 0x7fc8d6, Plain, 0x3a8fb0).shiny(0.25),
        ),
        "Labridae" => (
            "wrasse",
            Plan {
                snout: 0.55,
                peduncle: 0.38,
                speed: 1.2,
                dorsal: [0.22, 0.86, 0.05],
                anal: [0.55, 0.86, 0.05],
                ..reef(0.2, 0.3, 0.13, Truncate, [1, 4])
            },
            look(0x2f8f6a, 0x9fd3a0, 0x3aa07f, Stripes(3), 0xc86fa6),
        ),
        "Scaridae" => (
            "parrotfish",
            Plan {
                snout: 0.25,
                peduncle: 0.4,
                dorsal: [0.25, 0.86, 0.06],
                anal: [0.58, 0.86, 0.06],
                ..reef(0.4, 0.38, 0.16, Lunate, [1, 8])
            },
            look(0x2b8f8c, 0x60b8a4, 0x3a7fc0, Mottled, 0xd88cb4),
        ),
        "Serranidae" | "Epinephelidae" => (
            "grouper",
            Plan {
                speed: 0.4,
                depths: [2.0, 120.0],
                dorsal: [0.25, 0.82, 0.07],
                anal: [0.62, 0.82, 0.07],
                ..fish(0.6, 0.32, 0.17, Rounded, Reef, [2.0, 120.0], [1, 1], 0.4)
            },
            look(0x5e4a38, 0x8c7a62, 0x4e3d2e, Spots(5), 0x2e2219),
        ),
        "Lutjanidae" => (
            "snapper",
            Plan {
                snout: 0.55,
                dorsal: [0.25, 0.85, 0.06],
                anal: [0.6, 0.82, 0.06],
                pectoral: 0.18,
                ..fish(0.45, 0.36, 0.14, Forked, Reef, [2.0, 120.0], [4, 30], 0.9)
            },
            look(0xa8453f, 0xe0a090, 0xc0605a, Plain, 0x7a2a26).shiny(0.2),
        ),
        "Caesionidae" => (
            "fusilier",
            fish(
                0.25,
                0.28,
                0.12,
                Forked,
                Pelagic,
                [1.0, 60.0],
                [20, 80],
                1.5,
            ),
            look(0x2f6db5, 0xe8eef2, 0xf2d23c, Fade, 0xf2d23c)
                .tail(0xf2c62a)
                .shiny(0.3),
        ),
        "Haemulidae" => (
            "grunt / sweetlips",
            reef(0.45, 0.38, 0.14, Truncate, [3, 20]),
            look(0x8d9090, 0xd8d8d2, 0x6e7070, Plain, 0x3a3a3a),
        ),
        "Lethrinidae" => (
            "emperor",
            Plan {
                snout: 0.6,
                ..reef(0.45, 0.38, 0.14, Forked, [2, 15])
            },
            look(0x8f9590, 0xd9dcd6, 0x9f8f80, Plain, 0x55595a).shiny(0.3),
        ),
        "Mullidae" => (
            "goatfish",
            fish(0.3, 0.25, 0.13, Forked, Demersal, [1.0, 60.0], [3, 20], 0.8),
            look(0xd3c49a, 0xf3eedf, 0xf0d050, Stripes(1), 0xf3c624),
        ),
        "Holocentridae" => (
            "squirrelfish / soldierfish",
            reef(0.25, 0.35, 0.13, Forked, [2, 20]),
            look(0xb8342a, 0xe98b7f, 0xd0453a, Stripes(4), 0xf0e0dc).shiny(0.25),
        ),
        "Balistidae" => (
            "triggerfish",
            Plan {
                peak: 0.45,
                dorsal: [0.45, 0.85, 0.12],
                dorsal2: [0.26, 0.32, 0.08],
                anal: [0.55, 0.85, 0.12],
                pectoral: 0.1,
                ..reef(0.35, 0.48, 0.12, Lunate, [1, 3])
            },
            look(0x2e3d55, 0x56647a, 0x354560, Plain, 0x101418),
        ),
        "Monacanthidae" => (
            "filefish",
            Plan {
                snout: 0.7,
                ..reef(0.3, 0.45, 0.08, Rounded, [1, 2])
            },
            look(0x7c7a5e, 0xb8b48c, 0x6f6c50, Mottled, 0x3e5f8a),
        ),
        "Tetraodontidae" | "Diodontidae" => (
            "pufferfish",
            Plan {
                peak: 0.45,
                snout: 0.2,
                speed: 0.4,
                dorsal: [0.7, 0.8, 0.07],
                anal: [0.72, 0.8, 0.06],
                pectoral: 0.1,
                ..reef(0.35, 0.38, 0.32, Rounded, [1, 1])
            },
            look(0x8a8d86, 0xe8e6da, 0x8a8d86, Spots(6), 0x15171a),
        ),
        "Ostraciidae" => (
            "boxfish",
            Plan {
                snout: 0.15,
                speed: 0.4,
                dorsal: [0.7, 0.78, 0.06],
                anal: [0.72, 0.8, 0.05],
                ..reef(0.25, 0.38, 0.32, Rounded, [1, 1])
            },
            look(0xe0c02a, 0xf0d860, 0xe0c02a, Spots(6), 0x111111),
        ),
        "Scorpaenidae" => (
            "scorpionfish / lionfish",
            Plan {
                speed: 0.2,
                dorsal: [0.15, 0.8, 0.22],
                pectoral: 0.35,
                ..fish(0.3, 0.33, 0.18, Rounded, Benthic, [1.0, 60.0], [1, 1], 0.2)
            },
            look(0x7a4a3a, 0xb08070, 0x8a5a48, Mottled, 0x3a2a22),
        ),
        "Muraenidae" => (
            "moray eel",
            Plan {
                shape: Shape::Eel,
                peak: 0.35,
                snout: 0.55,
                peduncle: 0.5,
                dorsal: [0.1, 1.0, 0.03],
                anal: [0.5, 1.0, 0.03],
                pectoral: 0.0,
                ..fish(1.2, 0.08, 0.07, Pointed, Benthic, [1.0, 50.0], [1, 1], 0.3)
            },
            look(0x5a4a32, 0x8a7a5a, 0x5a4a32, Mottled, 0x2a2016),
        ),
        "Ephippidae" => (
            "batfish / spadefish",
            Plan {
                dorsal: [0.25, 0.75, 0.3],
                anal: [0.45, 0.75, 0.3],
                ..fish(0.45, 0.8, 0.1, Truncate, Pelagic, [1.0, 40.0], [3, 20], 0.6)
            },
            look(0xb8bcbf, 0xe0e2e2, 0x9da0a2, Bars(2), 0x3a3a3c).shiny(0.4),
        ),
        "Siganidae" => (
            "rabbitfish",
            reef(0.3, 0.42, 0.12, Forked, [2, 10]),
            look(0x9b8a5a, 0xd8cfa8, 0xd8c050, Spots(7), 0xf0e6c0),
        ),
        "Zanclidae" => (
            "Moorish idol",
            Plan {
                snout: 0.8,
                dorsal: [0.25, 0.6, 0.45],
                anal: [0.45, 0.75, 0.15],
                ..reef(0.18, 0.65, 0.08, Truncate, [1, 3])
            },
            look(0xf2f0e8, 0xf6f4ee, 0xf2d040, Bars(2), 0x121212)
                .tail(0x121212)
                .face(0xf2a040),
        ),
        "Kyphosidae" => (
            "sea chub",
            reef(0.45, 0.4, 0.14, Forked, [5, 30]),
            look(0x7a8288, 0xb8bec0, 0x6a7276, Stripes(5), 0x9aa2a8).shiny(0.3),
        ),
        "Apogonidae" => (
            "cardinalfish",
            Plan {
                speed: 0.3,
                ..reef(0.08, 0.35, 0.12, Forked, [10, 40])
            },
            look(0xc89a6a, 0xe8d0b0, 0xd0a070, Stripes(3), 0x5a3a2a),
        ),
        "Priacanthidae" => (
            "bigeye",
            reef(0.3, 0.4, 0.12, Truncate, [1, 10]),
            look(0xc03a2c, 0xe07060, 0xc03a2c, Plain, 0x801a14).shiny(0.2),
        ),
        "Pempheridae" => (
            "sweeper",
            reef(0.15, 0.45, 0.1, Forked, [20, 80]),
            look(0xb07a4a, 0xe0c0a0, 0xa06a3a, Plain, 0x3a2a1a).shiny(0.4),
        ),
        "Cirrhitidae" => (
            "hawkfish",
            Plan {
                speed: 0.2,
                ..fish(0.1, 0.32, 0.12, Truncate, Benthic, [1.0, 40.0], [1, 1], 0.2)
            },
            look(0xd8b0a0, 0xf0e0d8, 0xd06050, Spots(6), 0xb02a20),
        ),
        "Fistulariidae" | "Aulostomidae" => (
            "cornetfish / trumpetfish",
            Plan {
                snout: 1.0,
                bill: 0.22,
                peak: 0.5,
                peduncle: 0.4,
                dorsal: [0.75, 0.8, 0.02],
                anal: [0.75, 0.8, 0.02],
                pectoral: 0.04,
                ..fish(1.0, 0.04, 0.04, Forked, Reef, [1.0, 60.0], [1, 3], 0.6)
            },
            look(0x7c8a72, 0xc8d0bc, 0x7c8a72, Plain, 0x3a5a7a),
        ),
        "Synodontidae" => (
            "lizardfish",
            Plan {
                speed: 0.1,
                ..fish(0.25, 0.15, 0.15, Forked, Benthic, [1.0, 60.0], [1, 1], 0.1)
            },
            look(0x9a8a6a, 0xe0d8c0, 0x9a8a6a, Saddles(5), 0x5a4a32),
        ),
        "Sphyraenidae" => (
            "barracuda",
            Plan {
                peak: 0.45,
                snout: 0.85,
                peduncle: 0.45,
                tail_span: 0.22,
                dorsal: [0.38, 0.48, 0.07],
                dorsal2: [0.62, 0.7, 0.06],
                anal: [0.62, 0.7, 0.06],
                pectoral: 0.08,
                ..fish(1.0, 0.14, 0.11, Forked, Pelagic, [1.0, 100.0], [1, 40], 0.8)
            },
            look(0x6f7f88, 0xdde3e6, 0x5a666c, Saddles(14), 0x2a3236).shiny(0.6),
        ),
        "Carangidae" => (
            "jack / trevally",
            Plan {
                peduncle: 0.16,
                tail_span: 0.4,
                dorsal: [0.4, 0.85, 0.05],
                dorsal2: [0.28, 0.36, 0.06],
                anal: [0.55, 0.85, 0.05],
                pectoral: 0.25,
                ..fish(0.7, 0.36, 0.12, Lunate, Pelagic, [1.0, 150.0], [3, 40], 1.4)
            },
            look(0x6a7a84, 0xe4e9ea, 0x5a6a74, Plain, 0x2a3640).shiny(0.7),
        ),
        "Scombridae" => (
            "tuna / mackerel",
            Plan {
                peak: 0.42,
                snout: 0.65,
                peduncle: 0.1,
                tail_span: 0.42,
                dorsal: [0.25, 0.4, 0.1],
                dorsal2: [0.5, 0.58, 0.12],
                anal: [0.55, 0.62, 0.12],
                pectoral: 0.2,
                ..fish(1.0, 0.27, 0.22, Lunate, Pelagic, [0.0, 250.0], [5, 40], 1.8)
            },
            look(0x1b2a4a, 0xdfe3e8, 0x2a3550, Plain, 0x1b2a4a).shiny(0.8),
        ),
        "Coryphaenidae" => (
            "mahi-mahi",
            Plan {
                peak: 0.15,
                snout: 0.1,
                peduncle: 0.2,
                dorsal: [0.05, 0.88, 0.08],
                anal: [0.45, 0.88, 0.06],
                ..fish(1.0, 0.24, 0.09, Forked, Pelagic, [0.0, 85.0], [2, 10], 1.6)
            },
            look(0x2f7f6a, 0xe2c74b, 0x2a6f8a, Spots(8), 0x2a6fb0).shiny(0.6),
        ),
        "Istiophoridae" | "Xiphiidae" => (
            "billfish",
            Plan {
                snout: 0.9,
                bill: 0.2,
                peak: 0.35,
                peduncle: 0.1,
                tail_span: 0.35,
                dorsal: [0.22, 0.6, 0.12],
                anal: [0.6, 0.68, 0.06],
                pectoral: 0.15,
                ..fish(2.5, 0.15, 0.09, Lunate, Pelagic, [0.0, 200.0], [1, 1], 1.0)
            },
            look(0x1c3557, 0xd8dde3, 0x203a60, Saddles(12), 0x5a8ad0).shiny(0.6),
        ),
        "Belonidae" | "Hemiramphidae" => (
            "needlefish",
            Plan {
                snout: 1.0,
                bill: 0.18,
                peak: 0.5,
                dorsal: [0.72, 0.82, 0.04],
                anal: [0.7, 0.8, 0.04],
                pectoral: 0.05,
                ..fish(0.8, 0.06, 0.05, Forked, Surface, [0.0, 5.0], [2, 15], 1.2)
            },
            look(0x2e6b6b, 0xdfe8e6, 0x2e6b6b, Plain, 0x2a4a8a).shiny(0.7),
        ),
        "Exocoetidae" => (
            "flying fish",
            Plan {
                snout: 0.35,
                pectoral: 0.55,
                ..fish(0.25, 0.16, 0.12, Forked, Surface, [0.0, 10.0], [5, 30], 1.5)
            },
            look(0x2b4f8a, 0xe0e6ec, 0x6a8ab0, Plain, 0x2b4f8a).shiny(0.7),
        ),
        "Clupeidae" | "Engraulidae" | "Dussumieriidae" => (
            "sardine / herring / anchovy",
            Plan {
                peak: 0.4,
                snout: 0.5,
                peduncle: 0.25,
                dorsal: [0.42, 0.55, 0.07],
                anal: [0.7, 0.8, 0.04],
                pectoral: 0.12,
                ..fish(
                    0.18,
                    0.22,
                    0.12,
                    Forked,
                    Pelagic,
                    [0.0, 100.0],
                    [60, 200],
                    1.6,
                )
            },
            look(0x3b5f73, 0xe6ecef, 0x8aa0aa, Plain, 0x2a4a5a).shiny(0.85),
        ),
        "Mugilidae" => (
            "mullet",
            Plan {
                snout: 0.2,
                ..fish(0.4, 0.22, 0.16, Forked, Surface, [0.0, 20.0], [5, 40], 1.0)
            },
            look(0x5a6a6e, 0xdfe4e4, 0x7a8a8e, Stripes(5), 0x4a5a5e).shiny(0.5),
        ),
        "Gadidae" | "Merlucciidae" | "Lotidae" => (
            "cod / hake",
            Plan {
                peak: 0.3,
                snout: 0.4,
                dorsal: [0.25, 0.78, 0.07],
                anal: [0.45, 0.78, 0.05],
                ..fish(
                    0.8,
                    0.22,
                    0.15,
                    Truncate,
                    Demersal,
                    [10.0, 600.0],
                    [1, 30],
                    0.6,
                )
            },
            look(0x6a6a4a, 0xe5e1cc, 0x6a6a4a, Spots(6), 0x4a4430),
        ),
        "Pleuronectidae" | "Bothidae" | "Paralichthyidae" | "Soleidae" | "Scophthalmidae"
        | "Cynoglossidae" => (
            "flatfish",
            Plan {
                shape: Shape::Flat,
                speed: 0.1,
                dorsal: [0.1, 0.85, 0.08],
                anal: [0.3, 0.85, 0.08],
                ..fish(0.4, 0.45, 0.06, Rounded, Benthic, [5.0, 200.0], [1, 1], 0.1)
            },
            look(0x7a6a4e, 0xf0ece0, 0x7a6a4e, Spots(6), 0xd06a2a),
        ),
        "Salmonidae" => (
            "salmon / trout",
            fish(0.8, 0.24, 0.13, Forked, Pelagic, [0.0, 100.0], [3, 30], 1.2),
            look(0x4a5b5e, 0xe6e8e6, 0x5a6668, Spots(9), 0x1a1c1e).shiny(0.7),
        ),
        "Sparidae" => (
            "sea bream",
            fish(
                0.35,
                0.4,
                0.13,
                Forked,
                Demersal,
                [1.0, 150.0],
                [3, 30],
                0.9,
            ),
            look(0xa6aeb3, 0xe8ecee, 0x9aa0a6, Stripes(5), 0x6a7278).shiny(0.6),
        ),
        "Moronidae" | "Sciaenidae" | "Centropomidae" | "Polynemidae" => (
            "bass / drum",
            fish(
                0.6,
                0.27,
                0.13,
                Forked,
                Demersal,
                [0.0, 100.0],
                [2, 20],
                0.8,
            ),
            look(0x7a8088, 0xe2e4e6, 0x6a7076, Plain, 0x3a4048).shiny(0.5),
        ),
        "Molidae" => (
            "ocean sunfish",
            Plan {
                peak: 0.4,
                snout: 0.2,
                peduncle: 0.7,
                tail_span: 0.5,
                dorsal: [0.55, 0.7, 0.45],
                anal: [0.55, 0.7, 0.45],
                pectoral: 0.06,
                ..fish(1.8, 0.9, 0.12, Truncate, Pelagic, [0.0, 400.0], [1, 1], 0.3)
            },
            look(0x8b9096, 0xd0d4d6, 0x7b8086, Mottled, 0xb8bcc0),
        ),
        "Chanidae" | "Megalopidae" | "Elopidae" | "Albulidae" | "Rachycentridae" => (
            "milkfish / tarpon / cobia",
            fish(1.0, 0.22, 0.13, Forked, Pelagic, [0.0, 50.0], [1, 10], 1.0),
            look(0x5a6a7a, 0xe8ecee, 0x6a7a8a, Plain, 0x3a4a5a).shiny(0.8),
        ),
        // Sharks.
        "Carcharhinidae" | "Triakidae" | "Hemigaleidae" => (
            "requiem shark",
            shark(1.8, Reef, [1.0, 150.0], [1, 4]),
            look(0x6b7278, 0xe8e8e6, 0x5e656a, Plain, 0x15171a),
        ),
        "Lamnidae" | "Odontaspididae" => (
            "mackerel shark",
            Plan {
                depth: 0.22,
                width: 0.19,
                snout: 0.65,
                peduncle: 0.12,
                tail: Lunate,
                tail_span: 0.35,
                ..shark(3.5, Pelagic, [0.0, 300.0], [1, 1])
            },
            look(0x5a636b, 0xeceeee, 0x50585e, Plain, 0x2a2e30),
        ),
        "Sphyrnidae" => (
            "hammerhead shark",
            Plan {
                dorsal: [0.3, 0.4, 0.18],
                ..shark(2.5, Pelagic, [1.0, 300.0], [1, 20])
            },
            look(0x7a7d72, 0xe6e6e0, 0x6a6d62, Plain, 0x2a2c26),
        ),
        "Rhincodontidae" => (
            "whale shark",
            Plan {
                depth: 0.18,
                width: 0.2,
                snout: 0.05,
                dorsal: [0.4, 0.52, 0.09],
                speed: 0.15,
                ..shark(8.0, Surface, [0.0, 100.0], [1, 1])
            },
            look(0x3b4d5c, 0xd8dcdf, 0x3b4d5c, Spots(9), 0xe8eef2),
        ),
        "Ginglymostomatidae" | "Hemiscylliidae" | "Stegostomatidae" | "Scyliorhinidae"
        | "Orectolobidae" => (
            "nurse / carpet shark",
            Plan {
                depth: 0.14,
                width: 0.17,
                snout: 0.1,
                peduncle: 0.3,
                tail_span: 0.15,
                dorsal: [0.5, 0.6, 0.08],
                dorsal2: [0.63, 0.7, 0.07],
                speed: 0.2,
                ..shark(2.2, Benthic, [1.0, 70.0], [1, 1])
            },
            look(0x7a6a50, 0xc8b898, 0x6a5a40, Plain, 0x3a2e20),
        ),
        "Alopiidae" | "Cetorhinidae" | "Squalidae" | "Hexanchidae" => (
            "shark",
            shark(2.5, Pelagic, [0.0, 500.0], [1, 3]),
            look(0x5a5e66, 0xd8dadc, 0x50545a, Plain, 0x2a2c30),
        ),
        // Rays and skates.
        "Mobulidae" => (
            "manta / devil ray",
            ray(3.5, Pelagic, [0.0, 120.0], [1, 6], 0.3),
            look(0x1e1f22, 0xeef0ef, 0x1e1f22, Plain, 0xd8dcd8),
        ),
        "Myliobatidae" | "Aetobatidae" | "Rhinopteridae" => (
            "eagle ray",
            ray(2.0, Pelagic, [1.0, 60.0], [1, 8], 0.4),
            look(0x26292e, 0xeef0ef, 0x26292e, Spots(7), 0xe8eaea),
        ),
        "Dasyatidae" | "Urotrygonidae" | "Gymnuridae" | "Torpedinidae" | "Narcinidae"
        | "Rajidae" | "Urolophidae" => (
            "stingray / skate",
            ray(1.0, Benthic, [1.0, 100.0], [1, 1], 0.2),
            look(0x7a6e5d, 0xe8e4dc, 0x7a6e5d, Mottled, 0x5a4e3e),
        ),
        "Rhinidae" | "Rhinobatidae" | "Glaucostegidae" | "Pristidae" => (
            "wedgefish / guitarfish",
            Plan {
                depth: 0.1,
                width: 0.25,
                snout: 0.6,
                speed: 0.3,
                ..shark(1.8, Benthic, [1.0, 70.0], [1, 1])
            },
            look(0x9a8a6a, 0xeae6dc, 0x8a7a5a, Spots(6), 0xe8e0c8),
        ),
        // Sea turtles.
        "Cheloniidae" => (
            "sea turtle",
            turtle(1.0),
            look(0x5b5236, 0xd8cfa0, 0x8a7e5e, Mottled, 0x8a6a3a),
        ),
        "Dermochelyidae" => (
            "leatherback turtle",
            Plan {
                habit: Pelagic,
                depths: [0.0, 300.0],
                ..turtle(1.8)
            },
            look(0x1d1f22, 0xb8bcbe, 0x2a2c30, Spots(8), 0xc8ccd0),
        ),
        // Whales, dolphins, porpoises; sirenians.
        "Delphinidae" => (
            "dolphin",
            cetacean(2.5, 0.8, 0.12, [4, 20], 1.2),
            look(0x5a6068, 0xd3d6d9, 0x50565e, Plain, 0x5a6068),
        ),
        "Phocoenidae" => (
            "porpoise",
            Plan {
                dorsal: [0.42, 0.52, 0.07],
                ..cetacean(1.6, 0.2, 0.07, [1, 6], 1.0)
            },
            look(0x3a3e44, 0xc8ccd0, 0x3a3e44, Plain, 0x3a3e44),
        ),
        "Balaenopteridae" | "Eschrichtiidae" | "Balaenidae" => (
            "baleen whale",
            Plan {
                depth: 0.16,
                peduncle: 0.08,
                tail_span: 0.28,
                dorsal: [0.66, 0.71, 0.04],
                pectoral: 0.12,
                ..cetacean(14.0, 0.3, 0.04, [1, 3], 0.25)
            },
            look(0x3a4048, 0xc0c4c8, 0x3a4048, Mottled, 0x55606a),
        ),
        "Physeteridae" | "Kogiidae" | "Ziphiidae" => (
            "toothed whale",
            Plan {
                depth: 0.22,
                peak: 0.25,
                dorsal: [0.62, 0.7, 0.03],
                ..cetacean(10.0, 0.0, 0.03, [1, 6], 0.25)
            },
            look(0x4a4744, 0x8a8682, 0x4a4744, Mottled, 0x5e5a56),
        ),
        "Dugongidae" | "Trichechidae" => (
            "dugong / manatee",
            Plan {
                depth: 0.26,
                width: 0.26,
                snout: 0.1,
                dorsal: [0.0; 3],
                pectoral: 0.1,
                habit: Reef,
                depths: [1.0, 20.0],
                ..cetacean(2.8, 0.1, 0.0, [1, 3], 0.3)
            },
            look(0x7a7068, 0x9a908a, 0x6a605a, Plain, 0x6a605a),
        ),
        "Phocidae" | "Otariidae" => (
            "seal / sea lion",
            Plan {
                depth: 0.22,
                width: 0.22,
                snout: 0.35,
                dorsal: [0.0; 3],
                pectoral: 0.14,
                ..cetacean(1.8, 0.35, 0.0, [1, 6], 1.0)
            },
            look(0x4a4038, 0x8a7a68, 0x3a322c, Spots(6), 0x2a2420),
        ),
        _ => return None,
    })
}

/// Known colouring (and size) of particular species; genus fallbacks.
fn species_look(scientific: &str) -> Option<(&'static str, Look, Option<f64>)> {
    let l = |back, belly, fin, coat, accent| look(back, belly, fin, coat, accent);
    Some(match scientific {
        "Acanthurus leucosternon" => (
            "powder blue tang",
            l(0x3f8fd0, 0x5aa6e0, 0xf2d23c, Plain, 0x101010)
                .face(0x141414)
                .tail(0x2a2a2a),
            Some(0.23),
        ),
        "Acanthurus lineatus" => (
            "lined surgeonfish",
            l(0xf0c419, 0xc8d8e8, 0x2a5aa0, Stripes(6), 0x2a5ab0),
            Some(0.3),
        ),
        "Acanthurus triostegus" => (
            "convict surgeonfish",
            l(0xd8dcc8, 0xeceee6, 0xc8ccb8, Bars(5), 0x16181a),
            Some(0.2),
        ),
        "Acanthurus coeruleus" => (
            "Atlantic blue tang",
            l(0x2f4fa8, 0x3a60b8, 0x2a48a0, Plain, 0x1a2a60),
            Some(0.25),
        ),
        "Paracanthurus hepatus" => (
            "palette surgeonfish",
            l(0x2f5fd0, 0x3a70e0, 0x1a1a2a, Fade, 0x12141c).tail(0xf2d23c),
            Some(0.25),
        ),
        "Zebrasoma flavescens" => (
            "yellow tang",
            l(0xf2d21c, 0xf6dc40, 0xf2d21c, Plain, 0xf2d21c),
            Some(0.18),
        ),
        "Zebrasoma scopas" | "Zebrasoma desjardinii" | "Zebrasoma veliferum" => (
            "sailfin tang",
            l(0x5a4a3a, 0x7a6a5a, 0x4a3a2a, Bars(6), 0xb8a070),
            Some(0.25),
        ),
        "Naso lituratus" | "Naso elegans" => (
            "orangespine unicornfish",
            l(0x3a3e44, 0x4a4e54, 0xf0a020, Plain, 0x2a2c30).face(0xd8b020),
            Some(0.35),
        ),
        "Ctenochaetus striatus" => (
            "striated surgeonfish",
            l(0x4a3e34, 0x5a4e44, 0x3a2e24, Stripes(8), 0x6a5a4a),
            Some(0.22),
        ),
        "Zanclus cornutus" => (
            "Moorish idol",
            l(0xf2f0e8, 0xf6f4ee, 0xf2d040, Bars(2), 0x121212)
                .tail(0x121212)
                .face(0xf2a040),
            Some(0.18),
        ),
        "Chaetodon auriga" => (
            "threadfin butterflyfish",
            l(0xeeeeea, 0xf4f4f0, 0xf2c419, Fade, 0xf2c419).eyebar(),
            Some(0.2),
        ),
        "Chaetodon trifasciatus" | "Chaetodon lunulatus" => (
            "melon butterflyfish",
            l(0xf0b030, 0xf8d060, 0xf0a020, Stripes(8), 0x7a5aa0).eyebar(),
            Some(0.13),
        ),
        "Chaetodon collare" => (
            "redtail butterflyfish",
            l(0x4a3a30, 0x5a4a40, 0x3a2a20, Bars(2), 0xeaeae0)
                .tail(0xd03028)
                .eyebar(),
            Some(0.15),
        ),
        "Chaetodon meyeri" | "Chaetodon ornatissimus" => (
            "scrawled butterflyfish",
            l(0xe8ecf0, 0xf0f2f4, 0xf2c419, Stripes(6), 0x16181a).eyebar(),
            Some(0.18),
        ),
        "Heniochus acuminatus" | "Heniochus diphreutes" | "Heniochus monoceros" => (
            "bannerfish",
            l(0xf2f2ee, 0xf6f6f2, 0xf2d040, Bars(2), 0x141414).tail(0xf2d040),
            Some(0.2),
        ),
        "Forcipiger flavissimus" | "Forcipiger longirostris" => (
            "longnose butterflyfish",
            l(0xf2d21c, 0xf6dc40, 0xf2d21c, Plain, 0x141414).face(0x2a2a2a),
            Some(0.18),
        ),
        "Pomacanthus imperator" => (
            "emperor angelfish",
            l(0x1f3a9a, 0x2a4ab0, 0xf2a020, Stripes(9), 0xf2d040)
                .tail(0xf2a020)
                .eyebar(),
            Some(0.35),
        ),
        "Pygoplites diacanthus" => (
            "regal angelfish",
            l(0xf0901c, 0xf6b040, 0xf2c040, Bars(7), 0x3a5ab8),
            Some(0.22),
        ),
        "Pomacanthus paru" | "Pomacanthus arcuatus" => (
            "French angelfish",
            l(0x1e1e22, 0x2a2a30, 0x1e1e22, Spots(8), 0xd8c040),
            Some(0.35),
        ),
        "Chromis viridis" | "Chromis atripectoralis" => (
            "blue-green chromis",
            l(0x5ac8c0, 0xa8eae0, 0x6ad0c8, Plain, 0x4ab0b0).shiny(0.3),
            Some(0.08),
        ),
        "Dascyllus aruanus" => (
            "humbug damselfish",
            l(0xf2f2ee, 0xf6f6f2, 0x121212, Bars(3), 0x121212),
            Some(0.08),
        ),
        "Dascyllus trimaculatus" => (
            "threespot dascyllus",
            l(0x141414, 0x1a1a1a, 0x141414, Spots(2), 0xeeeeee),
            Some(0.11),
        ),
        "Abudefduf vaigiensis" | "Abudefduf saxatilis" | "Abudefduf sexfasciatus" => (
            "sergeant major",
            l(0xe8e0a0, 0xf0f0e0, 0xd8d0a0, Bars(5), 0x16181a),
            Some(0.17),
        ),
        "Amphiprion ocellaris" | "Amphiprion percula" | "Amphiprion nigripes" => (
            "clownfish",
            l(0xf07a1c, 0xf89a40, 0xf07a1c, Bars(3), 0xf6f6f2),
            Some(0.09),
        ),
        "Thalassoma lunare" => (
            "moon wrasse",
            l(0x2fa07a, 0x5ac0a0, 0xc050a0, Bars(8), 0xc860a8).tail(0xf2d040),
            Some(0.25),
        ),
        "Thalassoma hardwicke" => (
            "sixbar wrasse",
            l(0x6ac0a0, 0xa0e0c8, 0xc060a0, Saddles(6), 0x16181a),
            Some(0.18),
        ),
        "Thalassoma bifasciatum" => (
            "bluehead wrasse",
            l(0x2fa07a, 0x5ac0a0, 0x3a8ad0, Bars(2), 0x16181a).face(0x2a5ad0),
            Some(0.15),
        ),
        "Cheilinus undulatus" => (
            "humphead wrasse",
            l(0x2f7a8a, 0x4a9aa0, 0x2a6a7a, Mottled, 0x6ab0a8),
            Some(1.2),
        ),
        "Labroides dimidiatus" => (
            "bluestreak cleaner wrasse",
            l(0x6aa0e0, 0xe8eef6, 0x4a80c0, Stripes(1), 0x121418),
            Some(0.1),
        ),
        "Bolbometopon muricatum" => (
            "bumphead parrotfish",
            l(0x3a6a5a, 0x5a8a78, 0x3a6a5a, Plain, 0x9ab0a0).face(0xc8b0a0),
            Some(1.0),
        ),
        "Chlorurus sordidus" | "Scarus ghobban" | "Scarus rubroviolaceus" | "Scarus frenatus" => (
            "parrotfish",
            l(0x2a9a8a, 0x5ab8a8, 0x3a7fc0, Mottled, 0xd88cb4),
            Some(0.4),
        ),
        "Cephalopholis argus" => (
            "peacock hind",
            l(0x2a2c34, 0x3a3c44, 0x2a2c34, Spots(9), 0x4a8ad8),
            Some(0.4),
        ),
        "Cephalopholis miniata" | "Plectropomus leopardus" | "Plectropomus laevis" => (
            "coral hind / coral trout",
            l(0xc8302a, 0xe06050, 0xc8302a, Spots(9), 0x4aa0e0),
            Some(0.45),
        ),
        "Variola louti" => (
            "yellow-edged lyretail",
            l(0xc83a3a, 0xe06a60, 0xc83a3a, Spots(8), 0xc08ae0).tail(0xf2c040),
            Some(0.6),
        ),
        "Epinephelus lanceolatus" => (
            "giant grouper",
            l(0x4a4a3a, 0x7a7a60, 0x4a4a3a, Mottled, 0xd8c040),
            Some(2.0),
        ),
        "Epinephelus itajara" => (
            "Atlantic goliath grouper",
            l(0x5a4a30, 0x8a7a5a, 0x5a4a30, Mottled, 0x2a2014),
            Some(2.0),
        ),
        "Epinephelus fuscoguttatus" | "Epinephelus malabaricus" | "Epinephelus tauvina" => (
            "brown-marbled grouper",
            l(0x6a5a3a, 0x9a8a6a, 0x5a4a2a, Mottled, 0x3a2a1a),
            Some(0.8),
        ),
        "Pseudanthias squamipinnis" | "Pseudanthias evansi" | "Pseudanthias tuka" => (
            "anthias",
            l(0xf07a5a, 0xf8a080, 0xf07a5a, Plain, 0xc05aa0),
            Some(0.1),
        ),
        "Lutjanus kasmira" | "Lutjanus bengalensis" => (
            "bluestripe snapper",
            l(0xf0c419, 0xf4e8b0, 0xf0c419, Stripes(4), 0x3a8ae0).shiny(0.2),
            Some(0.3),
        ),
        "Lutjanus bohar"
        | "Lutjanus gibbus"
        | "Lutjanus campechanus"
        | "Lutjanus argentimaculatus" => (
            "red snapper",
            l(0xb0383a, 0xe08a80, 0xa03030, Plain, 0x701a1a).shiny(0.2),
            Some(0.6),
        ),
        "Macolor niger" | "Macolor macularis" => (
            "black-and-white snapper",
            l(0x1e1e22, 0x9a9a9e, 0x1e1e22, Spots(4), 0xe8e8e8),
            Some(0.6),
        ),
        "Ocyurus chrysurus" => (
            "yellowtail snapper",
            l(0x5a6a9a, 0xe8e0d0, 0xf2d040, Stripes(1), 0xf2d040).tail(0xf2d040),
            Some(0.4),
        ),
        "Caesio teres" | "Caesio caerulaurea" | "Caesio lunaris" | "Pterocaesio tile" => (
            "fusilier",
            l(0x2f6db5, 0xe8eef2, 0xf2d23c, Fade, 0xf2d23c)
                .tail(0xf2c62a)
                .shiny(0.3),
            Some(0.3),
        ),
        "Plectorhinchus vittatus" | "Plectorhinchus lineatus" | "Plectorhinchus orientalis" => (
            "oriental sweetlips",
            l(0xeeeeea, 0xf4f4f0, 0xf2d040, Stripes(6), 0x16181a),
            Some(0.6),
        ),
        "Plectorhinchus chaetodonoides" => (
            "harlequin sweetlips",
            l(0x4a3a30, 0xe8e0d0, 0x4a3a30, Spots(6), 0xeeeeea),
            Some(0.6),
        ),
        "Haemulon sciurus" | "Haemulon flavolineatum" | "Haemulon plumierii" => (
            "grunt",
            l(0xf0c419, 0xf4e0a0, 0x6a6a6a, Stripes(7), 0x2a5ab0),
            Some(0.35),
        ),
        "Monotaxis grandoculis" => (
            "humpnose big-eye bream",
            l(0x9aa0a4, 0xe0e4e6, 0xd0b060, Saddles(3), 0x3a3e42).shiny(0.4),
            Some(0.5),
        ),
        "Mulloidichthys flavolineatus"
        | "Mulloidichthys vanicolensis"
        | "Mulloidichthys martinicus" => (
            "yellowstripe goatfish",
            l(0xdcd8c8, 0xf4f2ea, 0xf2d040, Stripes(1), 0xf2c419),
            Some(0.35),
        ),
        "Parupeneus macronemus" | "Parupeneus barberinus" | "Parupeneus multifasciatus" => (
            "goatfish",
            l(0xd88a7a, 0xf0d0c8, 0xd0a060, Saddles(3), 0x2a2224),
            Some(0.3),
        ),
        "Myripristis murdjan" | "Myripristis berndti" | "Myripristis vittata" => (
            "soldierfish",
            l(0xc8302a, 0xe8705e, 0xd0453a, Plain, 0xf0e0dc).shiny(0.35),
            Some(0.25),
        ),
        "Sargocentron spiniferum" | "Sargocentron diadema" | "Sargocentron caudimaculatum" => (
            "squirrelfish",
            l(0xc8302a, 0xe8705e, 0xd0453a, Stripes(5), 0xf0e0dc).shiny(0.3),
            Some(0.3),
        ),
        "Balistoides conspicillum" => (
            "clown triggerfish",
            l(0x121418, 0x1a1c20, 0x121418, Spots(5), 0xf0f0ea).face(0xf2c040),
            Some(0.35),
        ),
        "Balistoides viridescens" => (
            "titan triggerfish",
            l(0x5a6a5a, 0x8a9a80, 0x3a4a3a, Mottled, 0xd0b060),
            Some(0.6),
        ),
        "Rhinecanthus aculeatus" | "Rhinecanthus rectangulus" => (
            "Picasso triggerfish",
            l(0xb8a888, 0xeeeae0, 0x8a7a5a, Bars(3), 0x2a2a30),
            Some(0.25),
        ),
        "Odonus niger" => (
            "redtoothed triggerfish",
            l(0x2a2a7a, 0x3a3a90, 0x3a4ab0, Plain, 0x2a2a7a),
            Some(0.4),
        ),
        "Balistapus undulatus" => (
            "orange-lined triggerfish",
            l(0x2a4a3a, 0x3a5a4a, 0xf08a1c, Stripes(8), 0xf08a1c),
            Some(0.3),
        ),
        "Melichthys niger" | "Melichthys indicus" => (
            "black durgon",
            l(0x14161a, 0x1a1c20, 0x14161a, Plain, 0x6a8ad0),
            Some(0.35),
        ),
        "Arothron nigropunctatus" => (
            "blackspotted puffer",
            l(0x8a8d86, 0xd8d0a0, 0x8a8d86, Spots(5), 0x15171a),
            Some(0.3),
        ),
        "Arothron hispidus" | "Arothron stellatus" => (
            "white-spotted puffer",
            l(0x6a6a5a, 0xe8e6da, 0x6a6a5a, Spots(9), 0xf0f0ea),
            Some(0.4),
        ),
        "Diodon hystrix" | "Diodon holocanthus" => (
            "porcupinefish",
            l(0x9a8a6a, 0xeae6da, 0x9a8a6a, Spots(9), 0x2a2420),
            Some(0.5),
        ),
        "Ostracion cubicus" | "Ostracion meleagris" => (
            "boxfish",
            l(0xe0c02a, 0xf0d860, 0xe0c02a, Spots(6), 0x111111),
            Some(0.3),
        ),
        "Aluterus scriptus" => (
            "scrawled filefish",
            l(0x8a8a6a, 0xc8c8a8, 0x6a6a50, Spots(8), 0x2a7ad0),
            Some(0.8),
        ),
        "Pterois volitans" | "Pterois miles" | "Pterois antennata" | "Pterois radiata" => (
            "lionfish",
            l(0xb84a30, 0xeee0d0, 0xc06a50, Bars(9), 0xf4eee6),
            Some(0.35),
        ),
        "Gymnothorax javanicus" => (
            "giant moray",
            l(0x6a5a32, 0x9a8a5a, 0x6a5a32, Spots(9), 0x1a1408),
            Some(2.2),
        ),
        "Gymnothorax meleagris" => (
            "whitemouth moray",
            l(0x4a3a2a, 0x5a4a3a, 0x4a3a2a, Spots(9), 0xf0f0ea),
            Some(1.0),
        ),
        "Gymnothorax flavimarginatus" | "Gymnothorax funebris" => (
            "moray eel",
            l(0x5a6a2a, 0x7a8a4a, 0x5a6a2a, Mottled, 0x2a3010),
            Some(1.2),
        ),
        "Platax teira" | "Platax orbicularis" | "Platax pinnatus" => (
            "batfish",
            l(0xb8bcbf, 0xe0e2e2, 0x9da0a2, Bars(2), 0x3a3a3c).shiny(0.4),
            Some(0.5),
        ),
        "Sphyraena barracuda" => (
            "great barracuda",
            l(0x5f6f78, 0xdde3e6, 0x4a5660, Saddles(16), 0x1e2428).shiny(0.6),
            Some(1.3),
        ),
        "Sphyraena qenie" | "Sphyraena jello" | "Sphyraena putnamae" | "Sphyraena flavicauda" => (
            "schooling barracuda",
            l(0x6f7f88, 0xdde3e6, 0x5a666c, Saddles(18), 0x3a4246).shiny(0.6),
            Some(0.9),
        ),
        "Caranx ignobilis" => (
            "giant trevally",
            l(0x4a525a, 0xc8ccd0, 0x3a4048, Plain, 0x1a1e22).shiny(0.6),
            Some(1.2),
        ),
        "Caranx melampygus" => (
            "bluefin trevally",
            l(0x5a6a7a, 0xd8dcdc, 0x2a8ae0, Spots(9), 0x1e2a40)
                .tail(0x2a8ae0)
                .shiny(0.6),
            Some(0.7),
        ),
        "Caranx sexfasciatus" | "Caranx hippos" | "Caranx latus" => (
            "trevally",
            l(0x6a7a84, 0xe4e9ea, 0x5a6a74, Plain, 0x2a3640).shiny(0.7),
            Some(0.7),
        ),
        "Elagatis bipinnulata" => (
            "rainbow runner",
            l(0x2a5a9a, 0xe8e0b0, 0xd0c040, Stripes(3), 0x5ab0e0).shiny(0.6),
            Some(0.9),
        ),
        "Seriola dumerili" | "Seriola lalandi" | "Seriola rivoliana" => (
            "amberjack",
            l(0x5a6878, 0xe6e2d8, 0xc8b060, Stripes(1), 0xd8b040).shiny(0.6),
            Some(1.2),
        ),
        "Gnathanodon speciosus" => (
            "golden trevally",
            l(0xd0b040, 0xf0e0a0, 0xd0b040, Bars(8), 0x2a2a2a).shiny(0.6),
            Some(0.7),
        ),
        "Thunnus albacares" => (
            "yellowfin tuna",
            l(0x15234a, 0xdfe3e8, 0xf2d020, Stripes(1), 0xe0c020)
                .tail(0x2a3550)
                .shiny(0.85),
            Some(1.4),
        ),
        "Thunnus obesus" => (
            "bigeye tuna",
            l(0x15234a, 0xdfe3e8, 0xe0c040, Plain, 0x15234a).shiny(0.85),
            Some(1.4),
        ),
        "Thunnus thynnus" | "Thunnus orientalis" | "Thunnus maccoyii" => (
            "bluefin tuna",
            l(0x101c3a, 0xdfe3e8, 0x2a3550, Plain, 0x101c3a).shiny(0.85),
            Some(2.0),
        ),
        "Thunnus alalunga" => (
            "albacore",
            l(0x1b2a4a, 0xdfe3e8, 0x2a3550, Plain, 0x1b2a4a).shiny(0.85),
            Some(1.0),
        ),
        "Katsuwonus pelamis" => (
            "skipjack tuna",
            l(0x1e2c58, 0xdfe3e8, 0x2a3550, Stripes(4), 0x3a4a6a).shiny(0.85),
            Some(0.6),
        ),
        "Euthynnus affinis" | "Euthynnus alletteratus" | "Auxis thazard" => (
            "little tuna",
            l(0x1e3058, 0xdfe3e8, 0x2a3550, Saddles(10), 0x0e1428).shiny(0.85),
            Some(0.6),
        ),
        "Gymnosarda unicolor" => (
            "dogtooth tuna",
            l(0x2a3a4a, 0xd0d4d8, 0x2a3a4a, Plain, 0x2a3a4a).shiny(0.7),
            Some(1.4),
        ),
        "Acanthocybium solandri" => (
            "wahoo",
            l(0x1e3a6a, 0xd8dee4, 0x1e3a6a, Bars(24), 0x2a5ab0).shiny(0.7),
            Some(1.6),
        ),
        "Scomberomorus commerson" | "Scomberomorus cavalla" | "Scomberomorus maculatus" => (
            "Spanish mackerel",
            l(0x3a4a6a, 0xdfe3e8, 0x3a4a6a, Saddles(20), 0x8a98a8).shiny(0.7),
            Some(1.2),
        ),
        "Rastrelliger kanagurta" | "Scomber scombrus" | "Scomber japonicus" | "Scomber colias" => (
            "mackerel",
            l(0x2a6a6a, 0xe6ecef, 0x5a7a80, Saddles(14), 0x10181c).shiny(0.8),
            Some(0.3),
        ),
        "Coryphaena hippurus" => (
            "mahi-mahi",
            l(0x2f8f7a, 0xe8cc4a, 0x2a6fa0, Spots(8), 0x2a6fb0).shiny(0.6),
            Some(1.0),
        ),
        "Istiophorus platypterus" => (
            "sailfish",
            l(0x1c3557, 0xd8dde3, 0x203a80, Saddles(14), 0x5a8ad0).shiny(0.6),
            Some(2.5),
        ),
        "Makaira nigricans" | "Istiompax indica" | "Kajikia audax" => (
            "marlin",
            l(0x1c3557, 0xd8dde3, 0x203a60, Saddles(12), 0x5a8ad0).shiny(0.6),
            Some(3.0),
        ),
        "Xiphias gladius" => (
            "swordfish",
            l(0x3b2f3f, 0x9a9aa0, 0x3b2f3f, Plain, 0x3b2f3f).shiny(0.3),
            Some(2.5),
        ),
        "Sardinella longiceps"
        | "Sardinella gibbosa"
        | "Sardinella aurita"
        | "Sardina pilchardus"
        | "Sardinops sagax" => (
            "sardine",
            l(0x2f5a73, 0xe6ecef, 0x8aa0aa, Plain, 0x2a4a5a).shiny(0.9),
            Some(0.18),
        ),
        "Clupea harengus" | "Clupea pallasii" => (
            "herring",
            l(0x2f4f6a, 0xe6ecef, 0x8aa0aa, Plain, 0x2a4a5a).shiny(0.9),
            Some(0.28),
        ),
        "Engraulis encrasicolus"
        | "Engraulis mordax"
        | "Engraulis japonicus"
        | "Stolephorus indicus" => (
            "anchovy",
            l(0x6a8a90, 0xe8eef0, 0xa8b8bc, Stripes(1), 0xd8e0e4).shiny(0.9),
            Some(0.12),
        ),
        "Gadus morhua" | "Gadus macrocephalus" => (
            "Atlantic cod",
            l(0x6a6a4a, 0xe5e1cc, 0x6a6a4a, Spots(8), 0x4a4430),
            Some(0.9),
        ),
        "Melanogrammus aeglefinus" => (
            "haddock",
            l(0x5a5a62, 0xdadcde, 0x4a4a52, Plain, 0x16181a),
            Some(0.5),
        ),
        "Pollachius virens" | "Pollachius pollachius" => (
            "saithe / pollack",
            l(0x2a3a3a, 0xc8ccc8, 0x2a3a3a, Plain, 0x2a3a3a).shiny(0.4),
            Some(0.8),
        ),
        "Pleuronectes platessa" => (
            "European plaice",
            l(0x7a6a4e, 0xf0ece0, 0x7a6a4e, Spots(6), 0xd06a2a),
            Some(0.4),
        ),
        "Hippoglossus hippoglossus" | "Hippoglossus stenolepis" => (
            "halibut",
            l(0x4a4a3a, 0xf0ece0, 0x4a4a3a, Mottled, 0x3a3a2a),
            Some(1.5),
        ),
        "Salmo salar" => (
            "Atlantic salmon",
            l(0x4a5b5e, 0xe6e8e6, 0x5a6668, Spots(9), 0x1a1c1e).shiny(0.7),
            Some(0.8),
        ),
        "Sparus aurata" => (
            "gilthead seabream",
            l(0x8a98a0, 0xe8ecee, 0x8a98a0, Plain, 0xd8b040)
                .face(0x8a98a0)
                .shiny(0.6),
            Some(0.35),
        ),
        "Diplodus sargus" | "Diplodus vulgaris" | "Diplodus puntazzo" => (
            "white seabream",
            l(0xa8b0b4, 0xe8ecee, 0x8a9296, Bars(5), 0x3a3e42).shiny(0.5),
            Some(0.3),
        ),
        "Dicentrarchus labrax" => (
            "European seabass",
            l(0x6a7880, 0xe6eaec, 0x6a7880, Plain, 0x3a4248).shiny(0.6),
            Some(0.6),
        ),
        "Mola mola" | "Mola alexandrini" => (
            "ocean sunfish",
            l(0x8b9096, 0xd0d4d6, 0x7b8086, Mottled, 0xb8bcc0),
            Some(1.8),
        ),
        "Rachycentron canadum" => (
            "cobia",
            l(0x3a2e24, 0xd8d0c0, 0x3a2e24, Stripes(2), 0xd8d0c0),
            Some(1.2),
        ),
        "Megalops atlanticus" | "Megalops cyprinoides" => (
            "tarpon",
            l(0x4a5a6a, 0xe8ecee, 0x5a6a7a, Plain, 0x3a4a5a).shiny(0.9),
            Some(1.5),
        ),
        "Chanos chanos" => (
            "milkfish",
            l(0x5a7080, 0xe8ecee, 0x6a8090, Plain, 0x3a4a5a).shiny(0.85),
            Some(1.0),
        ),
        // Sharks.
        "Carcharhinus melanopterus" => (
            "blacktip reef shark",
            l(0x8a8678, 0xeceae4, 0x8a8678, Plain, 0x101010),
            Some(1.4),
        ),
        "Carcharhinus amblyrhynchos" => (
            "grey reef shark",
            l(0x6b7278, 0xe8e8e6, 0x5e656a, Plain, 0x202426),
            Some(1.6),
        ),
        "Carcharhinus albimarginatus" => (
            "silvertip shark",
            l(0x5a646e, 0xe8e8e6, 0x5a646e, Plain, 0xf0f0f0),
            Some(2.2),
        ),
        "Triaenodon obesus" => (
            "whitetip reef shark",
            l(0x6e6e6a, 0xdcdcd8, 0x6e6e6a, Plain, 0xf0f0f0),
            Some(1.5),
        ),
        "Galeocerdo cuvier" => (
            "tiger shark",
            l(0x6a6a60, 0xe8e6e0, 0x6a6a60, Saddles(12), 0x3a3a34),
            Some(3.5),
        ),
        "Carcharhinus leucas" => (
            "bull shark",
            l(0x6a6e70, 0xe8e8e6, 0x5e6264, Plain, 0x2a2c2e),
            Some(2.4),
        ),
        "Carcharhinus limbatus" | "Carcharhinus brevipinna" => (
            "blacktip shark",
            l(0x7a7a72, 0xeceae4, 0x7a7a72, Plain, 0x101010),
            Some(1.6),
        ),
        "Carcharhinus perezi" => (
            "Caribbean reef shark",
            l(0x6b7278, 0xe8e8e6, 0x5e656a, Plain, 0x202426),
            Some(2.0),
        ),
        "Carcharhinus longimanus" => (
            "oceanic whitetip shark",
            l(0x6e6a62, 0xe6e4de, 0x6e6a62, Plain, 0xf0f0f0),
            Some(2.5),
        ),
        "Carcharhinus falciformis" => (
            "silky shark",
            l(0x5e6670, 0xe6e8ea, 0x5e6670, Plain, 0x3a4048).shiny(0.2),
            Some(2.2),
        ),
        "Prionace glauca" => (
            "blue shark",
            l(0x2a4a9a, 0xeef0f2, 0x2a4a9a, Plain, 0x2a4a9a),
            Some(2.5),
        ),
        "Negaprion brevirostris" | "Negaprion acutidens" => (
            "lemon shark",
            l(0x9a9070, 0xe8e4d0, 0x9a9070, Plain, 0x6a6048),
            Some(2.5),
        ),
        "Carcharodon carcharias" => (
            "white shark",
            l(0x5a636b, 0xf2f2f0, 0x50585e, Plain, 0x2a2e30),
            Some(4.5),
        ),
        "Isurus oxyrinchus" => (
            "shortfin mako",
            l(0x2a4a8a, 0xf0f2f4, 0x2a4a8a, Plain, 0x1a2a5a).shiny(0.2),
            Some(3.0),
        ),
        "Sphyrna lewini" | "Sphyrna mokarran" | "Sphyrna zygaena" => (
            "hammerhead shark",
            l(0x7a7d72, 0xe6e6e0, 0x6a6d62, Plain, 0x2a2c26),
            Some(3.0),
        ),
        "Rhincodon typus" => (
            "whale shark",
            l(0x34465a, 0xd8dcdf, 0x34465a, Spots(9), 0xe8eef2),
            Some(8.0),
        ),
        "Nebrius ferrugineus" | "Ginglymostoma cirratum" => (
            "nurse shark",
            l(0x7a6a50, 0xc8b898, 0x6a5a40, Plain, 0x3a2e20),
            Some(2.5),
        ),
        "Stegostoma tigrinum" | "Stegostoma fasciatum" => (
            "zebra shark",
            l(0xc8b48a, 0xeae0c8, 0xc8b48a, Spots(8), 0x3a2e20),
            Some(2.0),
        ),
        "Cetorhinus maximus" => (
            "basking shark",
            l(0x4a4a48, 0x8a8a86, 0x4a4a48, Mottled, 0x6a6a66),
            Some(7.0),
        ),
        "Squalus acanthias" => (
            "spiny dogfish",
            l(0x5a5e62, 0xe0e2e2, 0x5a5e62, Spots(6), 0xe8e8e8),
            Some(0.9),
        ),
        // Rays.
        "Mobula alfredi" | "Mobula birostris" => (
            "manta ray",
            l(0x1a1b1e, 0xeef0ef, 0x1a1b1e, Plain, 0xd8dcd8),
            Some(4.0),
        ),
        "Aetobatus narinari" | "Aetobatus ocellatus" => (
            "spotted eagle ray",
            l(0x22252a, 0xeef0ef, 0x22252a, Spots(7), 0xf0f2f2),
            Some(2.0),
        ),
        "Taeniura lymma" => (
            "bluespotted ribbontail ray",
            l(0x9c8a5c, 0xe8e4dc, 0x2a8ad8, Spots(7), 0x2a8ad8),
            Some(0.35),
        ),
        "Hypanus americanus" | "Dasyatis americana" => (
            "southern stingray",
            l(0x5a5a50, 0xe8e6e0, 0x5a5a50, Plain, 0x3a3a32),
            Some(1.2),
        ),
        "Taeniurops meyeni" | "Pateobatis fai" | "Himantura uarnak" | "Urogymnus granulatus" => (
            "stingray",
            l(0x5a5650, 0xe8e6e0, 0x5a5650, Mottled, 0x3a3630),
            Some(1.4),
        ),
        "Rhynchobatus djiddensis" | "Rhynchobatus australiae" => (
            "giant guitarfish",
            l(0x9a8a6a, 0xeae6dc, 0x8a7a5a, Spots(6), 0xe8e0c8),
            Some(2.2),
        ),
        "Raja clavata" | "Leucoraja naevus" | "Raja montagui" => (
            "skate",
            l(0x8a7a5a, 0xeee8dc, 0x8a7a5a, Spots(7), 0x4a3e2a),
            Some(0.7),
        ),
        // Turtles.
        "Chelonia mydas" => (
            "green turtle",
            l(0x5b5236, 0xd8cfa0, 0x8a7e5e, Mottled, 0x8a6a3a),
            Some(1.0),
        ),
        "Eretmochelys imbricata" => (
            "hawksbill turtle",
            l(0x6b4426, 0xe8d8a0, 0x7a6040, Mottled, 0xc8913b),
            Some(0.8),
        ),
        "Caretta caretta" => (
            "loggerhead turtle",
            l(0x7a4a2a, 0xe0c890, 0x8a6a4a, Mottled, 0x9a5a30),
            Some(0.95),
        ),
        "Lepidochelys olivacea" | "Lepidochelys kempii" => (
            "ridley turtle",
            l(0x5f6448, 0xd8d0a8, 0x6a6a50, Plain, 0x4a4e38),
            Some(0.65),
        ),
        "Dermochelys coriacea" => (
            "leatherback turtle",
            l(0x1d1f22, 0xb8bcbe, 0x2a2c30, Spots(8), 0xc8ccd0),
            Some(1.8),
        ),
        // Marine mammals.
        "Tursiops truncatus" => (
            "common bottlenose dolphin",
            l(0x5a6068, 0xd3d6d9, 0x50565e, Plain, 0x5a6068),
            Some(2.8),
        ),
        "Tursiops aduncus" => (
            "Indo-Pacific bottlenose dolphin",
            l(0x60666e, 0xd8dade, 0x565c64, Plain, 0x5a6068),
            Some(2.5),
        ),
        "Stenella longirostris" => (
            "spinner dolphin",
            l(0x40464e, 0xe0e2e4, 0x40464e, Plain, 0x8a9098),
            Some(1.9),
        ),
        "Stenella attenuata" | "Stenella frontalis" => (
            "spotted dolphin",
            l(0x3e444c, 0xc8ccd0, 0x3e444c, Spots(8), 0xd8dadc),
            Some(2.0),
        ),
        "Stenella coeruleoalba" => (
            "striped dolphin",
            l(0x3e4650, 0xe6e8ea, 0x3e4650, Stripes(2), 0x2a3038),
            Some(2.2),
        ),
        "Delphinus delphis" | "Delphinus capensis" => (
            "common dolphin",
            l(0x2e343c, 0xe8e8e6, 0x2e343c, Fade, 0xb8a878),
            Some(2.0),
        ),
        "Grampus griseus" => (
            "Risso's dolphin",
            l(0xa0a4a8, 0xc8cacc, 0x8a8e92, Mottled, 0xe0e2e4),
            Some(3.2),
        ),
        "Globicephala macrorhynchus" | "Globicephala melas" => (
            "pilot whale",
            l(0x1e2024, 0x3a3e42, 0x1e2024, Plain, 0x1e2024),
            Some(5.0),
        ),
        "Orcinus orca" => (
            "orca",
            l(0x101114, 0xf0f2f2, 0x101114, Plain, 0xf0f2f2),
            Some(7.0),
        ),
        "Pseudorca crassidens" | "Peponocephala electra" | "Feresa attenuata" => (
            "false killer whale",
            l(0x1e2024, 0x3a3e42, 0x1e2024, Plain, 0x1e2024),
            Some(5.0),
        ),
        "Sousa chinensis" | "Sousa plumbea" | "Sotalia guianensis" => (
            "humpback dolphin",
            l(0x9a9090, 0xd8c8c4, 0x8a8080, Mottled, 0xe0b8b0),
            Some(2.5),
        ),
        "Megaptera novaeangliae" => (
            "humpback whale",
            l(0x26292e, 0xd8dadc, 0xe8eaea, Mottled, 0x40444a),
            Some(13.0),
        ),
        "Balaenoptera musculus" => (
            "blue whale",
            l(0x5d7384, 0x8a9aa6, 0x5d7384, Mottled, 0x8aa0b0),
            Some(24.0),
        ),
        "Balaenoptera physalus" => (
            "fin whale",
            l(0x3a4048, 0xe0e2e4, 0x3a4048, Plain, 0x3a4048),
            Some(19.0),
        ),
        "Balaenoptera edeni" | "Balaenoptera brydei" | "Balaenoptera borealis" => (
            "Bryde's / sei whale",
            l(0x3a4048, 0xc0c4c8, 0x3a4048, Plain, 0x3a4048),
            Some(13.0),
        ),
        "Balaenoptera acutorostrata" | "Balaenoptera bonaerensis" => (
            "minke whale",
            l(0x3a4048, 0xe0e2e4, 0x3a4048, Plain, 0x3a4048),
            Some(8.0),
        ),
        "Physeter macrocephalus" => (
            "sperm whale",
            l(0x4a4744, 0x6a6662, 0x4a4744, Mottled, 0x5e5a56),
            Some(12.0),
        ),
        "Dugong dugon" => (
            "dugong",
            l(0x7a7068, 0x9a908a, 0x6a605a, Plain, 0x6a605a),
            Some(2.7),
        ),
        _ => return None,
    })
}

/// Typical colouring of a genus when the species is not catalogued.
fn genus_look(genus: &str) -> Option<(&'static str, Look, Option<f64>)> {
    let l = |back, belly, fin, coat, accent| look(back, belly, fin, coat, accent);
    Some(match genus {
        "Acanthurus" => (
            "surgeonfish",
            l(0x4a4e58, 0x6a6e78, 0x3a3e48, Plain, 0x1a1c20).tail(0x6a7aa0),
            None,
        ),
        "Chaetodon" => (
            "butterflyfish",
            l(0xe8e4d8, 0xf2efe6, 0xf0c419, Fade, 0xf3c623).eyebar(),
            None,
        ),
        "Chromis" => (
            "chromis",
            l(0x4f8fb8, 0x9fd0de, 0x5f9fc6, Plain, 0x3a7fb0).shiny(0.25),
            None,
        ),
        "Scarus" | "Chlorurus" | "Hipposcarus" => (
            "parrotfish",
            l(0x2a9a8a, 0x5ab8a8, 0x3a7fc0, Mottled, 0xd88cb4),
            None,
        ),
        "Lutjanus" => (
            "snapper",
            l(0xa8453f, 0xe0a090, 0xc0605a, Plain, 0x7a2a26).shiny(0.2),
            None,
        ),
        "Carcharhinus" => (
            "requiem shark",
            l(0x6b7278, 0xe8e8e6, 0x5e656a, Plain, 0x15171a),
            None,
        ),
        "Caesio" | "Pterocaesio" => (
            "fusilier",
            l(0x2f6db5, 0xe8eef2, 0xf2d23c, Fade, 0xf2d23c)
                .tail(0xf2c62a)
                .shiny(0.3),
            Some(0.25),
        ),
        "Pseudanthias" => (
            "anthias",
            l(0xf07a5a, 0xf8a080, 0xf07a5a, Plain, 0xc05aa0),
            Some(0.1),
        ),
        "Myripristis" => (
            "soldierfish",
            l(0xc8302a, 0xe8705e, 0xd0453a, Plain, 0xf0e0dc).shiny(0.35),
            None,
        ),
        "Dascyllus" => (
            "dascyllus",
            l(0xf2f2ee, 0xf6f6f2, 0x121212, Bars(3), 0x121212),
            None,
        ),
        "Amphiprion" => (
            "anemonefish",
            l(0xf07a1c, 0xf89a40, 0xf07a1c, Bars(3), 0xf6f6f2),
            Some(0.1),
        ),
        "Pterois" => (
            "lionfish",
            l(0xb84a30, 0xeee0d0, 0xc06a50, Bars(9), 0xf4eee6),
            None,
        ),
        "Thunnus" => (
            "tuna",
            l(0x15234a, 0xdfe3e8, 0x2a3550, Plain, 0x15234a).shiny(0.85),
            None,
        ),
        "Stenella" => (
            "dolphin",
            l(0x40464e, 0xd8dadc, 0x40464e, Plain, 0x8a9098),
            None,
        ),
        _ => return None,
    })
}

/// A species to draw: identity, body plan and colouring.
#[derive(Clone, Debug, PartialEq)]
pub struct Species {
    pub scientific: String,
    pub common: String,
    pub family: String,
    /// The colouring is the species' own (else its genus' or family's).
    pub own_colours: bool,
    pub plan: Plan,
    pub look: Look,
    /// Occurrence records in the area (how common it is in the data).
    pub records: u64,
}
impl Species {
    /// From an OBIS checklist record; None for groups without a model.
    pub fn from_record(scientific: &str, family: &str, records: u64) -> Option<Self> {
        let (family_common, mut plan, family_look) = self::family(family)?;
        let own = species_look(scientific);
        let genus = scientific.split(' ').next().unwrap_or("");
        let (common, look, length) = match own.or_else(|| genus_look(genus)) {
            Some((c, l, len)) => (c.to_string(), l, len),
            None => (family_common.to_string(), family_look, None),
        };
        if let Some(len) = length {
            plan.length = len;
        }
        // Genus-level adjustments of the family plan.
        match genus {
            "Caesio" | "Pterocaesio" => {
                plan.habit = Pelagic;
                plan.group = [20, 80];
                plan.speed = 1.5;
            }
            "Pseudanthias" | "Chromis" => plan.group = [15, 60],
            // Anemonefish live in small groups at their anemone; humbugs
            // and dascyllus hover in small groups over one coral head.
            "Amphiprion" | "Premnas" => {
                plan.group = [2, 5];
                plan.speed = 0.5;
            }
            "Dascyllus" => plan.group = [3, 15],
            "Cheilinus" | "Bolbometopon" => plan.depth = 0.38,
            "Sphyrna" => plan.group = [1, 12],
            "Megaptera" => plan.pectoral = 0.31,
            "Orcinus" | "Globicephala" | "Pseudorca" => {
                plan.snout = 0.15;
                plan.dorsal[2] = if genus == "Orcinus" { 0.2 } else { 0.08 };
            }
            "Grampus" => plan.snout = 0.1,
            "Rhincodon" | "Cetorhinus" => plan.habit = Surface,
            "Istiophorus" => plan.dorsal = [0.18, 0.62, 0.3],
            "Xiphias" => {
                plan.bill = 0.3;
                plan.dorsal = [0.28, 0.36, 0.14];
            }
            "Thalassoma" | "Halichoeres" | "Coris" => plan.group = [1, 6],
            "Lutjanus" if scientific == "Lutjanus kasmira" => plan.group = [10, 60],
            "Sphyraena" if scientific != "Sphyraena barracuda" => plan.group = [15, 80],
            "Sphyraena" => plan.group = [1, 1],
            "Pterois" => plan.habit = Reef,
            _ => {}
        }
        Some(Self {
            scientific: scientific.to_string(),
            common,
            family: family.to_string(),
            own_colours: own.is_some(),
            plan,
            look,
            records,
        })
    }
    /// Swimming as the renderer animates it: (locomotion mode, amplitude in
    /// body lengths, tail-beat angular frequency at cruise (rad/s), body
    /// waves per length).
    pub fn swim(&self) -> (u8, f64, f64, f64) {
        let p = &self.plan;
        let (mode, amp, k) = match p.shape {
            Shape::Fish | Shape::Shark => {
                let stiff = matches!(p.tail, Lunate) && p.peduncle < 0.2;
                (0, if stiff { 0.05 } else { 0.08 }, 5.5)
            }
            Shape::Eel => (1, 0.07, 9.0),
            Shape::Ray => (2, 0.12, 3.0),
            Shape::Cetacean => (3, 0.05, 4.0),
            Shape::Turtle => (4, 0.12, 0.0),
            Shape::Flat => (3, 0.05, 6.0),
        };
        // Tail-beat frequency scales with speed and falls with size.
        let hz = ((0.4 + 1.1 * p.speed) * (0.3 / p.length).powf(0.3)).clamp(0.12, 6.0);
        (mode, amp, hz * std::f64::consts::TAU, k)
    }
}

/// Picks the species to draw from a checklist ((scientific, family,
/// records) most recorded first): those with a model, at most `limit`, no
/// more than three per family.
pub fn choose(records: &[(String, String, u64)], limit: usize) -> Vec<Species> {
    let mut out: Vec<Species> = vec![];
    for (name, family, n) in records {
        if out.len() >= limit {
            break;
        }
        if out.iter().filter(|s| &s.family == family).count() >= 3
            || out.iter().any(|s| &s.scientific == name)
        {
            continue;
        }
        if let Some(s) = Species::from_record(name, family, *n) {
            out.push(s);
        }
    }
    out
}

/// Representative tropical reef and open-sea species, used where no
/// occurrence records are available (the template scene, offline).
pub fn representative() -> Vec<Species> {
    [
        ("Chromis viridis", "Pomacentridae"),
        ("Acanthurus lineatus", "Acanthuridae"),
        ("Chaetodon auriga", "Chaetodontidae"),
        ("Chlorurus sordidus", "Scaridae"),
        ("Lutjanus kasmira", "Lutjanidae"),
        ("Caranx melampygus", "Carangidae"),
        ("Zanclus cornutus", "Zanclidae"),
        ("Carcharhinus melanopterus", "Carcharhinidae"),
        ("Chelonia mydas", "Cheloniidae"),
        ("Taeniura lymma", "Dasyatidae"),
    ]
    .iter()
    .filter_map(|(n, f)| Species::from_record(n, f, 0))
    .collect()
}

// ------------------------------------------------------------ meshes

fn hash(a: i64, b: i64) -> f64 {
    let mut z = (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    z ^= z >> 31;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 29;
    (z >> 11) as f64 / (1u64 << 53) as f64
}
fn mix3(a: [f32; 3], b: [f32; 3], t: f64) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0) as f32;
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Body colour at a point: `s` along the body (0 snout .. 1 tail tip), `v`
/// up the side (-1 belly .. 1 back), `a` around the body (radians).
fn paint(look: &Look, s: f64, v: f64, a: f64, body_end: f64) -> [f32; 3] {
    let mut c = mix3(look.belly, look.back, smooth(-0.45, 0.55, v));
    let u = ((s - 0.12) / (body_end - 0.12)).clamp(0.0, 1.0);
    let inside = s > 0.12 && s < body_end;
    match look.coat {
        Plain => {}
        Bars(n) if inside => {
            if (u * n as f64 + 0.25).fract() < 0.42 {
                c = look.accent;
            }
        }
        Saddles(n) if inside => {
            if (u * n as f64 + 0.25).fract() < 0.45 && v > 0.05 {
                c = mix3(c, look.accent, smooth(0.05, 0.4, v));
            }
        }
        Stripes(n) => {
            let w = ((v + 1.0) * 0.5 * n as f64 + 0.3).fract();
            if inside && w < 0.38 {
                c = look.accent;
            }
        }
        Spots(k) => {
            let cs = s * k as f64 * 2.6;
            let ca = a / std::f64::consts::TAU * k as f64 * 3.0;
            let (i, j) = (cs.floor() as i64, ca.floor() as i64);
            let (fx, fy) = (cs.fract() - 0.5, ca.fract() - 0.5);
            let jitter = hash(i, j + 77) - 0.5;
            if inside && hash(i, j) < 0.7 && (fx + jitter * 0.3).hypot(fy) < 0.3 {
                c = look.accent;
            }
        }
        Mottled => {
            let n = hash((s * 9.0) as i64, (a * 2.2) as i64) * 0.6
                + hash((s * 21.0) as i64, (a * 5.0) as i64 + 9) * 0.4;
            c = mix3(c, look.accent, smooth(0.5, 0.8, n));
        }
        Fade => c = mix3(c, look.accent, smooth(0.35, 0.7, u)),
        _ => {}
    }
    if let Some(face) = look.face
        && s < 0.2
    {
        c = mix3(c, face, smooth(0.2, 0.12, s));
    }
    if look.eye_bar && (s - 0.1).abs() < 0.03 {
        c = [0.02, 0.02, 0.02];
    }
    c
}

fn push(out: &mut Vec<DrawVertex>, p: Vec3, n: Vec3, colour: [f32; 3], rough: f32, metal: f32) {
    out.push(DrawVertex {
        position: [p.x as f32, p.y as f32, p.z as f32],
        normal: [n.x as f32, n.y as f32, n.z as f32],
        uv: [p.x as f32, p.z as f32],
        albedo: colour,
        roughness: rough,
        metallic: metal,
        kind: 3.0,
    });
}
/// Emits a triangle wound so that its front face is the side its vertex
/// normals point to (the shader flips normals on back faces).
fn emit(out: &mut Vec<DrawVertex>, v: [(Vec3, Vec3, [f32; 3]); 3], rough: f32, metal: f32) {
    let face = (v[1].0 - v[0].0).cross(v[2].0 - v[0].0);
    let order = if face.dot(v[0].1 + v[1].1 + v[2].1) < 0.0 {
        [0, 2, 1]
    } else {
        [0, 1, 2]
    };
    for k in order {
        push(out, v[k].0, v[k].1, v[k].2, rough, metal);
    }
}
fn tri(out: &mut Vec<DrawVertex>, p: [Vec3; 3], colour: [f32; 3], rough: f32) {
    let n = (p[1] - p[0]).cross(p[2] - p[0]);
    let n = if n.length() > 1e-12 {
        n.normalized()
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    for q in p {
        push(out, q, n, colour, rough, 0.0);
    }
}

/// A lofted body: rings of (x, half width, half height, centre z) from the
/// snout to the tail; smooth normals; per-vertex colours.
fn loft(
    out: &mut Vec<DrawVertex>,
    rings: &[(f64, f64, f64, f64)],
    segs: usize,
    colour: &dyn Fn(usize, f64) -> [f32; 3],
    metal: f32,
) {
    let ring_point =
        |r: &(f64, f64, f64, f64), a: f64| Vec3::new(r.0, r.1 * a.cos(), r.3 + r.2 * a.sin());
    let angle = |j: usize| j as f64 * std::f64::consts::TAU / segs as f64;
    let normal = |i: usize, j: usize| {
        let a = angle(j);
        let r = &rings[i];
        let prev = &rings[i.saturating_sub(1)];
        let next = &rings[(i + 1).min(rings.len() - 1)];
        let along = ring_point(next, a) - ring_point(prev, a);
        let around = ring_point(r, a + 0.05) - ring_point(r, a - 0.05);
        let mut n = along.cross(around);
        let radial = Vec3::new(0.0, a.cos(), a.sin());
        if n.length() < 1e-9 {
            n = radial + Vec3::new(if i == 0 { 1.0 } else { -1.0 }, 0.0, 0.0);
        }
        if n.dot(radial) < 0.0 && n.dot(radial).abs() > 1e-9 {
            n = n * -1.0;
        }
        n.normalized()
    };
    for i in 0..rings.len() - 1 {
        for j in 0..segs {
            let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let v = |(ri, sj): (usize, usize)| {
                let a = angle(sj % segs);
                (
                    ring_point(&rings[ri], a),
                    normal(ri, sj % segs),
                    colour(ri, a),
                )
            };
            let q = corners.map(v);
            emit(out, [q[0], q[1], q[2]], 0.4, metal);
            emit(out, [q[0], q[2], q[3]], 0.4, metal);
        }
    }
}

/// A fin membrane between a base line and a tip line (double-sided).
fn membrane(out: &mut Vec<DrawVertex>, base: &[Vec3], tip: &[Vec3], colour: [f32; 3]) {
    for k in 0..base.len().saturating_sub(1) {
        tri(out, [base[k], base[k + 1], tip[k + 1]], colour, 0.55);
        tri(out, [base[k], tip[k + 1], tip[k]], colour, 0.55);
    }
}

/// Body half sizes at `s` (fraction from the snout): (half width, half
/// height).
fn section(plan: &Plan, s: f64, end: f64) -> (f64, f64) {
    let s0 = plan.bill;
    let u = ((s - s0) / (end - s0)).clamp(0.0, 1.0);
    let profile = |peak: f64| {
        if u <= peak {
            let q = u / peak;
            (1.0 - (1.0 - q) * (1.0 - q))
                .max(0.0)
                .powf(0.3 + 1.1 * plan.snout)
        } else {
            let q = (u - peak) / (1.0 - peak);
            plan.peduncle + (1.0 - plan.peduncle) * 0.5 * (1.0 + (std::f64::consts::PI * q).cos())
        }
    };
    let taper = if matches!(plan.tail, Pointed) && plan.shape != Shape::Ray {
        1.0 - smooth(0.6, 1.0, u)
    } else {
        1.0
    };
    (
        plan.width * 0.5 * profile(plan.peak * 0.9) * taper,
        plan.depth * 0.5 * profile(plan.peak) * taper,
    )
}

/// Model-space mesh for a species (snout at +x 0.5, length 1, +z up);
/// `detail` 1 for close views, 0 for distant ones.
pub fn mesh(species: &Species, detail: u8) -> Vec<DrawVertex> {
    let mut out = vec![];
    match species.plan.shape {
        Shape::Ray => ray_mesh(&mut out, species, detail),
        Shape::Turtle => turtle_mesh(&mut out, species, detail),
        _ => fish_mesh(&mut out, species, detail),
    }
    if species.plan.shape == Shape::Flat {
        // Lies on its side: the eyed side up, the blind side white below.
        for v in &mut out {
            let [x, y, z] = v.position;
            v.position = [x, -z, y * 1.6];
            let [nx, ny, nz] = v.normal;
            v.normal = [nx, -nz, ny];
            if v.normal[2] < -0.2 {
                v.albedo = species.look.belly;
            }
        }
    }
    out
}

fn fish_mesh(out: &mut Vec<DrawVertex>, sp: &Species, detail: u8) {
    let plan = &sp.plan;
    let look = &sp.look;
    let (count, segs) = if detail > 0 { (26, 14) } else { (10, 8) };
    let end = match (plan.shape, plan.tail) {
        (_, Pointed) => 1.0,
        (Shape::Shark, _) => 0.74,
        (Shape::Cetacean, _) => 0.9,
        _ => plan.bill + (1.0 - plan.bill) * 0.82,
    };
    let s0 = plan.bill;
    let stations: Vec<f64> = (0..count)
        .map(|i| {
            let t = i as f64 / (count - 1) as f64;
            s0 + (end - s0) * t.powf(1.25)
        })
        .collect();
    let rings: Vec<(f64, f64, f64, f64)> = stations
        .iter()
        .map(|&s| {
            let (hw, hd) = section(plan, s, end);
            // Many fish are deeper above the midline than below it.
            let camber = if plan.shape == Shape::Fish {
                hd * 0.08
            } else {
                0.0
            };
            (0.5 - s, hw, hd, camber)
        })
        .collect();
    loft(
        out,
        &rings,
        segs,
        &|i, a| paint(look, stations[i], a.sin(), a, end),
        look.metallic,
    );
    // Bill or rostrum.
    if plan.bill > 0.0 {
        let (hw, hd) = section(plan, s0 + 0.02, end);
        let r = hw.min(hd).max(0.004) * 0.6;
        let bill = [(0.5, 0.0005, 0.0005, 0.0), (0.5 - s0, r, r, 0.0)];
        loft(out, &bill, 6, &|_, _| look.back, look.metallic);
    }
    let at = |s: f64| section(plan, s, end);
    let fin_colour = look.fin;
    // Dorsal and anal fins: membranes swept back from the body outline.
    let mut fin = |spec: [f64; 3], up: f64, colour: [f32; 3], falcate: bool| {
        if spec[2] <= 0.0 || spec[1] <= spec[0] {
            return;
        }
        let n = if detail > 0 { 6 } else { 3 };
        let mut base = vec![];
        let mut tip = vec![];
        for k in 0..=n {
            let q = k as f64 / n as f64;
            let s = spec[0] + (spec[1] - spec[0]) * q;
            let (_, hd) = at(s.min(end));
            let h = if falcate {
                if q < 0.35 {
                    q / 0.35
                } else {
                    ((1.0 - q) / 0.65).powf(1.6)
                }
            } else {
                (std::f64::consts::PI * q).sin().powf(0.6) * 0.8 + 0.2 * (1.0 - q)
            } * spec[2];
            let sweep = if falcate { 0.9 } else { 0.35 };
            base.push(Vec3::new(0.5 - s, 0.0, up * hd * 0.95));
            tip.push(Vec3::new(0.5 - s - h * sweep, 0.0, up * (hd * 0.95 + h)));
        }
        membrane(out, &base, &tip, colour);
    };
    let falcate = matches!(plan.shape, Shape::Shark | Shape::Cetacean);
    fin(plan.dorsal, 1.0, fin_colour, falcate);
    if detail > 0 {
        fin(plan.dorsal2, 1.0, fin_colour, falcate);
        fin(plan.anal, -1.0, fin_colour, falcate);
    }
    // Shark fin tips (blacktip, whitetip, silvertip) in the accent colour.
    // Pectoral fins: paddles on fish, wings on sharks and cetaceans.
    if plan.pectoral > 0.0 {
        let s = plan.bill + 0.22 * (1.0 - plan.bill);
        let (hw, hd) = at(s);
        let len = plan.pectoral;
        for side in [-1.0, 1.0] {
            let root = Vec3::new(0.5 - s, side * hw * 0.9, -hd * 0.35);
            let (out_dir, down) = match plan.shape {
                Shape::Shark | Shape::Cetacean => (0.85, -0.3),
                _ if plan.pectoral > 0.3 => (0.9, 0.05),
                _ => (0.45, -0.15),
            };
            let back = Vec3::new(-0.6, side * out_dir, down).normalized();
            let chord = (len * 0.45).max(0.02);
            let a = root;
            let b = root - Vec3::new(chord, 0.0, 0.0);
            let tip = root + back * len;
            tri(out, [a, b, tip], fin_colour, 0.55);
        }
    }
    // Caudal fin.
    let (_, hp) = at(end);
    let tail_x = 0.5 - end;
    let span = plan.tail_span;
    let tc = look.tail;
    let fan = |out: &mut Vec<DrawVertex>, outline: &[(f64, f64)], horizontal: bool| {
        let centre = Vec3::new(tail_x + 0.01, 0.0, 0.0);
        let point = |(dx, h): (f64, f64)| {
            if horizontal {
                Vec3::new(tail_x - dx, h, 0.0)
            } else {
                Vec3::new(tail_x - dx, 0.0, h)
            }
        };
        for w in outline.windows(2) {
            tri(out, [centre, point(w[0]), point(w[1])], tc, 0.55);
        }
    };
    let rest = 1.0 - end;
    match plan.tail {
        Forked => fan(
            out,
            &[
                (0.0, hp),
                (rest, span * 0.5),
                (rest * 0.55, 0.0),
                (rest, -span * 0.5),
                (0.0, -hp),
            ],
            false,
        ),
        Lunate => fan(
            out,
            &[
                (0.0, hp),
                (rest * 0.7, span * 0.35),
                (rest, span * 0.5),
                (rest * 0.5, span * 0.15),
                (rest * 0.4, 0.0),
                (rest * 0.5, -span * 0.15),
                (rest, -span * 0.5),
                (rest * 0.7, -span * 0.35),
                (0.0, -hp),
            ],
            false,
        ),
        Rounded => {
            let pts: Vec<(f64, f64)> = (0..=8)
                .map(|k| {
                    let a = std::f64::consts::FRAC_PI_2 - k as f64 * std::f64::consts::PI / 8.0;
                    (
                        rest * a.cos().max(0.0) * 0.95,
                        (span * 0.45).max(hp) * a.sin(),
                    )
                })
                .collect();
            fan(out, &pts, false)
        }
        Truncate => fan(
            out,
            &[
                (0.0, hp),
                (rest * 0.95, span * 0.42),
                (rest, 0.0),
                (rest * 0.95, -span * 0.42),
                (0.0, -hp),
            ],
            false,
        ),
        Heterocercal => fan(
            out,
            &[
                (0.0, hp),
                (rest * 0.6, span * 0.35),
                (rest, span * 0.55),
                (rest * 0.75, span * 0.2),
                (rest * 0.45, -span * 0.02),
                (rest * 0.5, -span * 0.3),
                (0.0, -hp),
            ],
            false,
        ),
        Fluke => fan(
            out,
            &[
                (0.0, hp),
                (rest * 0.6, span * 0.4),
                (rest, span * 0.5),
                (rest * 0.75, span * 0.12),
                (rest * 0.7, 0.0),
                (rest * 0.75, -span * 0.12),
                (rest, -span * 0.5),
                (rest * 0.6, -span * 0.4),
                (0.0, -hp),
            ],
            true,
        ),
        Pointed => {}
    }
    // Hammerhead: the cephalofoil.
    if sp.family == "Sphyrnidae" && detail > 0 {
        let x = 0.5 - 0.05;
        let w = 0.13;
        let t = 0.012;
        let c = look.back;
        let quad = |out: &mut Vec<DrawVertex>, z: f64, colour: [f32; 3]| {
            let p = [
                Vec3::new(x + 0.04, -w, z),
                Vec3::new(x + 0.04, w, z),
                Vec3::new(x - 0.03, w, z),
                Vec3::new(x - 0.03, -w, z),
            ];
            tri(out, [p[0], p[1], p[2]], colour, 0.45);
            tri(out, [p[0], p[2], p[3]], colour, 0.45);
        };
        quad(out, t, c);
        quad(out, -t, look.belly);
    }
    // Eyes: dark pupil in a pale iris, on both sides of the head.
    if detail > 0 && (plan.shape != Shape::Cetacean || plan.length < 4.0) {
        let s = plan.bill
            + (end - plan.bill)
                * if plan.shape == Shape::Shark {
                    0.13
                } else {
                    0.1
                };
        let (hw, hd) = at(s);
        let r = (plan.depth * 0.11).clamp(0.008, 0.045);
        for side in [-1.0f64, 1.0] {
            let centre = Vec3::new(0.5 - s, side * (hw * 0.93 + 0.002), hd * 0.3);
            let normal = Vec3::new(0.0, side, 0.0);
            let ring = |radius: f64, k: usize| {
                let a = k as f64 * std::f64::consts::TAU / 8.0;
                centre + Vec3::new(radius * a.cos(), 0.0, radius * a.sin())
            };
            for k in 0..8 {
                let iris = [0.55, 0.5, 0.3];
                let (p0, p1) = (ring(r, k), ring(r, k + 1));
                let (q0, q1) = (ring(r * 0.55, k), ring(r * 0.55, k + 1));
                let lift = normal * 0.001;
                for (p, c) in [
                    ([centre + lift, q0 + lift, q1 + lift], [0.01, 0.01, 0.012]),
                    ([q0, p0, p1], iris),
                    ([q0, p1, q1], iris),
                ] {
                    emit(out, p.map(|v| (v, normal, c)), 0.1, 0.0);
                }
            }
        }
    }
}

fn ray_mesh(out: &mut Vec<DrawVertex>, sp: &Species, detail: u8) {
    let look = &sp.look;
    let manta = sp.family == "Mobulidae";
    let eagle = matches!(
        sp.family.as_str(),
        "Myliobatidae" | "Aetobatidae" | "Rhinopteridae"
    );
    let (radial, around) = if detail > 0 { (6, 28) } else { (3, 14) };
    // Disc outline radius by angle (0 = forward): pointed wings for mantas
    // and eagle rays, a rounded disc for stingrays and skates.
    let outline = |a: f64| {
        let (c, s) = (a.cos(), a.sin().abs());
        if manta || eagle {
            let wing = s.powf(3.0) * 0.5;
            let body = 0.22 * (1.0 - s).max(0.0) + 0.12;
            (wing + body * if c > 0.0 { 1.0 } else { 0.8 }).min(0.5)
        } else {
            0.3 + 0.12 * s - if c < 0.0 { 0.05 * c.abs() } else { 0.0 }
        }
    };
    let thickness = if manta { 0.05 } else { 0.035 };
    let point = |a: f64, t: f64, top: bool| {
        let r = outline(a) * t;
        let x = r * a.cos() * if manta || eagle { 0.55 } else { 0.85 };
        let y = r * a.sin();
        let z = thickness * (1.0 - t * t).max(0.0).sqrt() * if top { 1.0 } else { -0.45 };
        Vec3::new(x, y, z)
    };
    for top in [true, false] {
        for i in 0..radial {
            let (t0, t1) = (i as f64 / radial as f64, (i + 1) as f64 / radial as f64);
            for j in 0..around {
                let (a0, a1) = (
                    j as f64 * std::f64::consts::TAU / around as f64,
                    (j + 1) as f64 * std::f64::consts::TAU / around as f64,
                );
                let colour = |a: f64, t: f64| {
                    if !top {
                        look.belly
                    } else {
                        paint(
                            look,
                            0.5 - (t * a.cos()) * 0.5 + 0.0,
                            1.0,
                            a * 3.0 + t * 5.0,
                            0.95,
                        )
                    }
                };
                let corner = |(a, t): (f64, f64)| {
                    let n = Vec3::new(0.0, 0.0, if top { 1.0 } else { -1.0 })
                        + Vec3::new(a.cos(), a.sin(), 0.0) * (t * 0.6);
                    (point(a, t, top), n.normalized(), colour(a, t))
                };
                let q = [(a0, t0), (a0, t1), (a1, t1), (a1, t0)].map(corner);
                emit(out, [q[0], q[1], q[2]], 0.5, 0.0);
                emit(out, [q[0], q[2], q[3]], 0.5, 0.0);
            }
        }
    }
    // Tail: a thin whip (long on stingrays and eagle rays).
    let tail_len = if manta { 0.25 } else { 0.55 };
    let start = Vec3::new(-0.2, 0.0, 0.0);
    let tip = Vec3::new(-0.2 - tail_len, 0.0, 0.0);
    let rings = [
        (start.x, 0.012, 0.012, 0.0),
        (start.x - tail_len * 0.5, 0.006, 0.006, 0.0),
        (tip.x, 0.001, 0.001, 0.0),
    ];
    loft(out, &rings, 5, &|_, _| look.back, 0.0);
    // Manta cephalic lobes.
    if manta && detail > 0 {
        for side in [-1.0, 1.0] {
            let base = Vec3::new(0.2, side * 0.06, 0.0);
            tri(
                out,
                [
                    base,
                    base + Vec3::new(0.09, side * 0.015, -0.01),
                    base + Vec3::new(0.0, side * 0.03, -0.01),
                ],
                look.back,
                0.5,
            );
        }
    }
}

fn turtle_mesh(out: &mut Vec<DrawVertex>, sp: &Species, detail: u8) {
    let look = &sp.look;
    let (count, segs) = if detail > 0 { (14, 16) } else { (6, 8) };
    // Shell: a domed oval carapace over a flatter plastron.
    let rings: Vec<(f64, f64, f64, f64)> = (0..count)
        .map(|i| {
            let t = i as f64 / (count - 1) as f64;
            let x = 0.32 - t * 0.68;
            let w = (1.0 - (2.0 * t - 1.0).powi(2)).max(0.0).sqrt();
            (x, 0.3 * w, 0.11 * w, 0.02 * w)
        })
        .collect();
    loft(
        out,
        &rings,
        segs,
        &|i, a| {
            let t = i as f64 / (count - 1) as f64;
            if a.sin() < -0.2 {
                look.belly
            } else {
                // Scutes: a coarse mosaic of the shell colours.
                let k = hash((t * 5.0) as i64, (a * 1.6) as i64);
                mix3(look.back, look.accent, k * 0.8)
            }
        },
        0.0,
    );
    // Head and neck.
    let head = [
        (0.30, 0.05, 0.04, 0.0),
        (0.38, 0.06, 0.05, 0.01),
        (0.46, 0.05, 0.045, 0.01),
        (0.5, 0.001, 0.001, 0.005),
    ];
    loft(out, &head, 8, &|_, _| look.fin, 0.0);
    // Flippers: long front flippers, short rear ones.
    for side in [-1.0f64, 1.0] {
        let front = Vec3::new(0.18, side * 0.26, -0.02);
        tri(
            out,
            [
                front,
                front + Vec3::new(-0.1, 0.0, 0.0),
                front + Vec3::new(-0.22, side * 0.4, -0.03),
            ],
            look.fin,
            0.6,
        );
        let rear = Vec3::new(-0.26, side * 0.18, -0.02);
        tri(
            out,
            [
                rear,
                rear + Vec3::new(-0.06, 0.0, 0.0),
                rear + Vec3::new(-0.14, side * 0.13, -0.01),
            ],
            look.fin,
            0.6,
        );
    }
}

// ------------------------------------------------------------ living animals

#[derive(Clone, Debug)]
struct Member {
    offset: Vec3,
    phase: f64,
    scale: f64,
    tint: f32,
    swim_phase: f64,
}
#[derive(Clone, Debug)]
struct Group {
    species: usize,
    pos: Vec3,
    yaw: f64,
    pitch: f64,
    speed: f64,
    turn: f64,
    wander: f64,
    /// Resting on the bottom (benthic animals), seconds left.
    rest: f64,
    members: Vec<Member>,
}

/// One animal to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub species: usize,
    pub position: Vec3,
    pub heading: Vec3,
    pub length: f64,
    pub mode: u8,
    pub amplitude: f64,
    pub frequency: f64,
    pub wavenumber: f64,
    /// Phase to add to frequency × render time (continuous as speed varies).
    pub phase: f64,
    pub tint: [f32; 3],
    pub bend: f64,
}

/// What the population needs to know about the world.
pub struct Surroundings<'a> {
    /// Seabed elevation (m, negative below sea level).
    pub elevation: &'a dyn Fn(f64, f64) -> f64,
    /// Habitat class (`habitat::REEF`, `SEAGRASS`, 0 unknown).
    pub habitat: &'a dyn Fn(f64, f64) -> u8,
    pub camera: Vec3,
    /// How far animals are kept around the camera (m).
    pub range: f64,
    /// Individuals to keep around the camera.
    pub budget: usize,
}

/// Animals of the place's species living around the camera.
#[derive(Clone, Debug, Default)]
pub struct Population {
    pub species: Vec<Species>,
    groups: Vec<Group>,
    seed: u64,
}

/// How far from the camera a species is kept: as far as an animal of its
/// size can be made out (about 90 body lengths), within the water's range;
/// surface animals seen from above the water further.
fn reach(plan: &Plan, at: &Surroundings) -> f64 {
    let r = (plan.length * 90.0).clamp(12.0, at.range);
    if plan.habit == Surface && at.camera.z > 0.0 {
        r * 3.0
    } else {
        r
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

impl Population {
    pub fn new(species: Vec<Species>, seed: u64) -> Self {
        Self {
            species,
            groups: vec![],
            seed,
        }
    }
    pub fn len(&self) -> usize {
        self.groups.iter().map(|g| g.members.len()).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
    /// Species (index, number) currently within `distance` of a point.
    pub fn nearby(&self, p: Vec3, distance: f64) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = vec![];
        for g in &self.groups {
            if (g.pos - p).length() < distance {
                match out.iter_mut().find(|e| e.0 == g.species) {
                    Some(e) => e.1 += g.members.len(),
                    None => out.push((g.species, g.members.len())),
                }
            }
        }
        out.sort_by_key(|e| std::cmp::Reverse(e.1));
        out
    }
    pub fn clear(&mut self) {
        self.groups.clear();
    }

    /// A place for a new group of a species, or None if this spot does not
    /// suit it (land, too shallow or deep, wrong habitat).
    fn site(&self, plan: &Plan, at: &Surroundings, rng: &mut Rng, ring: bool) -> Option<Vec3> {
        let a = rng.range(0.0, std::f64::consts::TAU);
        let reach = reach(plan, at);
        let r = if ring {
            rng.range(reach * 0.6, reach)
        } else {
            rng.range((reach * 0.1).max(2.0), reach)
        };
        let (x, y) = (at.camera.x + r * a.cos(), at.camera.y + r * a.sin());
        let floor = (at.elevation)(x, y);
        let depth = -floor;
        let len = plan.length;
        if depth < (len * 0.6).max(0.8) {
            return None;
        }
        let [dmin, dmax] = plan.depths;
        let z = match plan.habit {
            Reef | Demersal => {
                if depth < dmin || depth > dmax {
                    return None;
                }
                if plan.habit == Reef {
                    let h = (at.habitat)(x, y);
                    // Reef fish over reef; elsewhere only where rocky and
                    // shallow (no habitat map), and less often.
                    if h != crate::habitat::REEF && (h != 0 || depth > 30.0 || rng.next() < 0.6) {
                        return None;
                    }
                }
                floor + rng.range(0.4, 5.0).min(depth * 0.6) + len * 0.4
            }
            Benthic => {
                if depth < dmin || depth > dmax {
                    return None;
                }
                floor + len * 0.06 + 0.05
            }
            Pelagic => {
                let top = dmin.max(1.0);
                let bottom = dmax.min(depth - 2.0 - len);
                if bottom < top {
                    return None;
                }
                // Mostly in the camera's part of the water column.
                let near = (-at.camera.z).clamp(top, bottom);
                -(near + rng.range(-12.0, 12.0)).clamp(top, bottom)
            }
            Surface => {
                if depth < 3.0 + len {
                    return None;
                }
                -rng.range(0.6 + len * 0.15, 4.0 + len * 0.3)
            }
        };
        Some(Vec3::new(x, y, z.min(-0.3 - plan.depth * len)))
    }

    fn spawn(&mut self, at: &Surroundings, rng: &mut Rng, ring: bool) -> bool {
        if self.species.is_empty() {
            return false;
        }
        // Pick a species weighted by how often it is recorded here.
        let weights: Vec<f64> = self
            .species
            .iter()
            .map(|s| (s.records as f64 + 4.0).sqrt() * if s.plan.length > 3.0 { 0.35 } else { 1.0 })
            .collect();
        let total: f64 = weights.iter().sum();
        let mut pick = rng.range(0.0, total);
        let mut index = 0;
        for (i, w) in weights.iter().enumerate() {
            if pick < *w {
                index = i;
                break;
            }
            pick -= w;
        }
        let plan = self.species[index].plan;
        let Some(pos) = self.site(&plan, at, rng, ring) else {
            return false;
        };
        let [lo, hi] = plan.group;
        let n = (rng.range(lo as f64, hi as f64 + 0.999) as usize).max(1);
        let polarized = lo >= 6;
        let spacing = plan.length * if polarized { 1.4 } else { 4.0 };
        let radius = spacing * (n as f64).cbrt() * 0.75;
        let members = (0..n)
            .map(|k| {
                let offset = if k == 0 {
                    Vec3::ZERO
                } else {
                    let u = Vec3::new(
                        rng.range(-1.0, 1.0),
                        rng.range(-1.0, 1.0),
                        rng.range(-1.0, 1.0),
                    );
                    Vec3::new(u.x * radius * 1.4, u.y * radius, u.z * radius * 0.45)
                };
                Member {
                    offset,
                    phase: rng.range(0.0, std::f64::consts::TAU),
                    scale: rng.range(0.8, 1.15),
                    tint: rng.range(0.88, 1.1) as f32,
                    swim_phase: rng.range(0.0, std::f64::consts::TAU),
                }
            })
            .collect();
        let benthic = plan.habit == Benthic;
        self.groups.push(Group {
            species: index,
            pos,
            yaw: rng.range(0.0, std::f64::consts::TAU),
            pitch: 0.0,
            speed: if benthic {
                0.0
            } else {
                plan.speed * plan.length
            },
            turn: 0.0,
            wander: rng.range(0.0, 100.0),
            rest: if benthic { rng.range(5.0, 60.0) } else { 0.0 },
            members,
        });
        true
    }

    /// Advances the animals by `dt` seconds: steering along the seabed and
    /// below the surface, schooling, avoiding the camera; animals far behind
    /// are replaced by new ones around the camera.
    pub fn update(&mut self, dt: f64, at: &Surroundings) {
        let dt = dt.clamp(0.0, 0.25);
        let mut rng =
            Rng(self.seed ^ (at.camera.x.to_bits().rotate_left(7)) ^ at.camera.y.to_bits());
        self.seed = self.seed.wrapping_add(0x51_7cc1_b727_220a);
        let species = &self.species;
        self.groups.retain(|g| {
            let d = g.pos - at.camera;
            Vec3::new(d.x, d.y, 0.0).length() < reach(&species[g.species].plan, at) * 1.6
        });
        // Fill up to the budget: anywhere in range at first, then on the
        // edge of the range so that animals do not appear in plain view.
        let first = self.groups.is_empty();
        let mut tries = if first { 400 } else { 12 };
        while self.len() < at.budget && tries > 0 {
            tries -= 1;
            self.spawn(at, &mut rng, !first);
        }
        for g in &mut self.groups {
            let plan = self.species[g.species].plan;
            let len = plan.length;
            if g.rest > 0.0 {
                g.rest -= dt;
                if g.rest <= 0.0 {
                    g.speed = plan.speed * len * 0.6;
                    g.wander = rng.range(0.0, 100.0);
                }
                continue;
            }
            g.wander += dt;
            let dir = Vec3::new(
                g.yaw.cos() * g.pitch.cos(),
                g.yaw.sin() * g.pitch.cos(),
                g.pitch.sin(),
            );
            // Wandering turns, plus avoiding what is ahead.
            let mut desired =
                0.35 * (g.wander * 0.37 + g.species as f64).sin() + 0.2 * (g.wander * 0.11).sin();
            let look = (g.speed * 2.5).max(len * 3.0).max(2.0);
            let ahead = g.pos + dir * look;
            let floor_ahead = (at.elevation)(ahead.x, ahead.y);
            let floor_here = (at.elevation)(g.pos.x, g.pos.y);
            let clearance = len * 0.5 + 0.4;
            let mut target_pitch = 0.0;
            if floor_ahead > ahead.z - clearance || -floor_ahead < plan.depths[0].min(1.5) {
                desired += 1.6;
                target_pitch = 0.35;
            }
            let preferred = match plan.habit {
                Reef | Demersal => floor_here + (2.0 + len).min(-floor_here * 0.5),
                Benthic => floor_here + len * 0.3 + 0.3,
                Pelagic => g.pos.z,
                Surface => -(0.8 + len * 0.2) + 0.6 * (g.wander * 0.25).sin(),
            };
            target_pitch += ((preferred - g.pos.z) * 0.12).clamp(-0.45, 0.45);
            if g.pos.z > -0.5 - plan.depth * len && plan.shape != Shape::Cetacean {
                target_pitch = target_pitch.min(-0.2);
            }
            // Keep out of the camera's way.
            let away = g.pos - at.camera;
            let mut boost = 1.0;
            if away.length() < 2.5 + len * 2.0 {
                let side = dir.x * away.y - dir.y * away.x;
                desired += if side > 0.0 { 2.0 } else { -2.0 };
                boost = 2.5;
            }
            let k = 1.0 - (-dt * 1.8).exp();
            g.turn += (desired - g.turn) * k;
            g.yaw += g.turn * dt * 0.6;
            g.pitch += (target_pitch - g.pitch) * k;
            g.pitch = g.pitch.clamp(-0.6, 0.6);
            let cruise = plan.speed * len * boost;
            g.speed += (cruise - g.speed) * k;
            g.pos = g.pos + dir * (g.speed * dt);
            let floor = (at.elevation)(g.pos.x, g.pos.y);
            let top = if plan.shape == Shape::Cetacean {
                -0.25
            } else {
                -0.3 - plan.depth * len * 0.6
            };
            let low = floor + len * 0.1 + 0.1;
            g.pos.z = g.pos.z.clamp(low, top.max(low));
            if plan.habit == Benthic && rng.next() < dt * 0.03 {
                g.rest = rng.range(10.0, 60.0);
                g.speed = 0.0;
            }
            let swim = self.species[g.species].swim();
            for m in &mut g.members {
                // Tail-beat phase integrated so its rate follows the speed.
                let rate = swim.2 * (0.6 + 0.4 * g.speed / (plan.speed * len).max(1e-3)).min(2.5);
                m.swim_phase = (m.swim_phase + rate * dt) % (std::f64::consts::TAU * 1000.0);
            }
        }
    }

    /// The animals to draw; `time` is the time the renderer animates with.
    /// Each stays between the seabed and the surface.
    pub fn poses(&self, time: f64, elevation: &dyn Fn(f64, f64) -> f64) -> Vec<Pose> {
        let mut out = vec![];
        for g in &self.groups {
            let sp = &self.species[g.species];
            let plan = sp.plan;
            let (mode, amp, freq, k) = sp.swim();
            let (cy, sy) = (g.yaw.cos(), g.yaw.sin());
            let (cp, spitch) = (g.pitch.cos(), g.pitch.sin());
            let forward = Vec3::new(cy * cp, sy * cp, spitch);
            let left = Vec3::new(-sy, cy, 0.0);
            let up = forward.cross(left);
            let polarized = plan.group[0] >= 6;
            for m in &g.members {
                let t = g.wander + m.phase;
                let drift = Vec3::new((t * 0.31).sin(), (t * 0.23).cos(), (t * 0.17).sin())
                    * (plan.length * 0.6);
                let o = m.offset + drift;
                let mut position = g.pos + forward * o.x + left * o.y + up * o.z;
                let low = elevation(position.x, position.y) + plan.length * m.scale * 0.12 + 0.05;
                let high: f64 = if plan.shape == Shape::Cetacean {
                    -0.2
                } else {
                    -0.25
                };
                position.z = position.z.clamp(low, high.max(low));
                let wobble = if polarized { 0.12 } else { 0.9 };
                let yaw = g.yaw + wobble * (t * 0.13).sin();
                let heading = Vec3::new(yaw.cos() * cp, yaw.sin() * cp, spitch);
                let amp = if g.rest > 0.0 { amp * 0.15 } else { amp };
                out.push(Pose {
                    species: g.species,
                    position,
                    heading,
                    length: plan.length * m.scale,
                    mode,
                    amplitude: amp,
                    frequency: freq,
                    wavenumber: k,
                    phase: m.swim_phase - freq * time,
                    tint: [m.tint, m.tint, m.tint],
                    bend: (g.turn * 0.04).clamp(-0.12, 0.12),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogue_maps_records_to_models() {
        let tang = Species::from_record("Acanthurus leucosternon", "Acanthuridae", 20).unwrap();
        assert_eq!(tang.common, "powder blue tang");
        assert!(tang.own_colours);
        assert_eq!(tang.plan.tail, Tail::Lunate);
        let other = Species::from_record("Acanthurus mata", "Acanthuridae", 3).unwrap();
        assert!(!other.own_colours);
        assert_eq!(other.common, "surgeonfish");
        assert!(Species::from_record("Gobiodon citrinus", "Gobiidae", 9).is_none());
        let records: Vec<(String, String, u64)> = [
            ("Acanthurus leucosternon", "Acanthuridae", 20),
            ("Acanthurus lineatus", "Acanthuridae", 18),
            ("Acanthurus triostegus", "Acanthuridae", 15),
            ("Acanthurus mata", "Acanthuridae", 12),
            ("Gobiodon citrinus", "Gobiidae", 10),
            ("Chelonia mydas", "Cheloniidae", 5),
        ]
        .iter()
        .map(|(a, b, n)| (a.to_string(), b.to_string(), *n))
        .collect();
        let chosen = choose(&records, 10);
        assert_eq!(chosen.len(), 4, "three per family plus the turtle");
        assert_eq!(chosen[3].common, "green turtle");
    }
    #[test]
    fn meshes_are_valid_for_every_shape() {
        for (name, family) in [
            ("Acanthurus leucosternon", "Acanthuridae"),
            ("Carcharhinus melanopterus", "Carcharhinidae"),
            ("Sphyrna lewini", "Sphyrnidae"),
            ("Mobula alfredi", "Mobulidae"),
            ("Taeniura lymma", "Dasyatidae"),
            ("Chelonia mydas", "Cheloniidae"),
            ("Tursiops aduncus", "Delphinidae"),
            ("Gymnothorax javanicus", "Muraenidae"),
            ("Pleuronectes platessa", "Pleuronectidae"),
            ("Istiophorus platypterus", "Istiophoridae"),
        ] {
            let s = Species::from_record(name, family, 1).unwrap();
            for detail in [0, 1] {
                let m = mesh(&s, detail);
                assert!(!m.is_empty() && m.len().is_multiple_of(3), "{name}");
                for v in &m {
                    assert!(
                        v.position.iter().all(|c| c.is_finite() && c.abs() < 0.9),
                        "{name} {:?}",
                        v.position
                    );
                    let n = v.normal;
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    assert!((len - 1.0).abs() < 1e-3, "{name} normal {n:?}");
                }
            }
            assert!(
                mesh(&s, 1).len() > mesh(&s, 0).len(),
                "{name} level of detail"
            );
        }
    }
    #[test]
    fn animals_live_in_the_water_around_the_camera() {
        let species = representative();
        let mut p = Population::new(species, 7);
        let elevation = |x: f64, _: f64| -12.0 - x * 0.01;
        let habitat = |_: f64, _: f64| crate::habitat::REEF;
        let mut camera = Vec3::new(0.0, 0.0, -5.0);
        for step in 0..600 {
            camera.x = step as f64 * 0.5;
            let at = Surroundings {
                elevation: &elevation,
                habitat: &habitat,
                camera,
                range: 50.0,
                budget: 150,
            };
            p.update(0.1, &at);
        }
        assert!(p.len() >= 100, "kept around the camera: {}", p.len());
        for pose in p.poses(60.0, &elevation) {
            let floor = elevation(pose.position.x, pose.position.y);
            assert!(pose.position.z < 0.0, "{pose:?}");
            assert!(pose.position.z > floor - 0.01, "{pose:?} floor {floor}");
            let d = pose.position - camera;
            assert!(
                Vec3::new(d.x, d.y, 0.0).length() < 50.0 * 1.6 + 40.0,
                "{pose:?}"
            );
            assert!((pose.heading.length() - 1.0).abs() < 1e-6);
        }
        // Nothing lives on land.
        let land = |_: f64, _: f64| 5.0;
        let mut dry = Population::new(representative(), 1);
        dry.update(
            0.1,
            &Surroundings {
                elevation: &land,
                habitat: &habitat,
                camera,
                range: 50.0,
                budget: 100,
            },
        );
        assert!(dry.is_empty());
    }
}
