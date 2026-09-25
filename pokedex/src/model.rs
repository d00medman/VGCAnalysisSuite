//! Domain types. These are the real interface for a pipeline caller; the JSON shape is
//! whatever serde derives from them.

use serde::{Deserialize, Serialize};

/// A complete submission for one regulation.
///
/// Moves and abilities are defined once here and referenced by name from each pokemon,
/// so a 1000-pokemon snapshot does not repeat move definitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Regulation name; must already exist (`pokedex regulation add`).
    pub regulation: String,
    #[serde(default)]
    pub abilities: Vec<AbilityRecord>,
    #[serde(default)]
    pub moves: Vec<MoveRecord>,
    #[serde(default)]
    pub pokemon: Vec<PokemonRecord>,
    /// Every item legal in this regulation. Complete when present: a stored item missing
    /// from it has its legality closed. `None` leaves legality untouched.
    #[serde(default)]
    pub items: Option<Vec<ItemRecord>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRecord {
    /// Slug, e.g. `focus-sash`.
    pub name: String,
    /// As battle text prints it, e.g. `Focus Sash`.
    pub display_name: String,
    pub category: ItemCategory,
    #[serde(default)]
    pub fling_power: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemCategory {
    MegaStone,
    Berry,
    Other,
}

impl ItemCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemCategory::MegaStone => "mega-stone",
            ItemCategory::Berry => "berry",
            ItemCategory::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityRecord {
    pub name: String,
    /// As battle text prints it, e.g. `Flower Veil`.
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveRecord {
    pub name: String,
    /// As battle text prints it, e.g. `King's Shield`. The slug cannot be turned back.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Elemental type name. Versioned: move types change (Bite went Normal -> Dark).
    #[serde(rename = "type")]
    pub type_name: String,
    pub damage_class: DamageClass,
    /// `None` = status move or variable power. Distinct from `Some(0)`.
    #[serde(default)]
    pub power: Option<i64>,
    /// `None` = never misses (Swift, Aerial Ace). Distinct from `Some(0)`.
    #[serde(default)]
    pub accuracy: Option<i64>,
    #[serde(default)]
    pub pp: Option<i64>,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub secondary_effect: Option<String>,
    #[serde(default)]
    pub effect_chance: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DamageClass {
    Physical,
    Special,
    Status,
}

impl DamageClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DamageClass::Physical => "physical",
            DamageClass::Special => "special",
            DamageClass::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LearnMethod {
    LevelUp,
    Machine,
    Egg,
    Tutor,
    Other,
}

impl LearnMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            LearnMethod::LevelUp => "level-up",
            LearnMethod::Machine => "machine",
            LearnMethod::Egg => "egg",
            LearnMethod::Tutor => "tutor",
            LearnMethod::Other => "other",
        }
    }
}

/// One pokemon in a snapshot.
///
/// # Absence is destructive — read this before constructing one
///
/// Ingest treats a snapshot as *complete*: anything missing from a provided set has its
/// interval closed. So the set-valued fields are `Option`, and the distinction matters:
///
/// - `None`          — not provided. Leave whatever is stored untouched.
/// - `Some(vec![])`  — explicitly empty. Close every open interval.
///
/// A scraper stage that fetched only stats must leave `learnset: None`, not
/// `Some(vec![])`, or it will wipe the learnset it never looked at.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PokemonRecord {
    pub national_dex_no: i64,
    /// `""` for the base form; `"alolan"`, `"mega-x"`, ... otherwise.
    #[serde(default)]
    pub form_slug: String,
    pub name: String,
    #[serde(default)]
    pub genus: Option<String>,
    #[serde(default)]
    pub height_dm: Option<i64>,
    #[serde(default)]
    pub weight_hg: Option<i64>,

    /// Present iff this row is a variant of another form. `None` = base form.
    #[serde(default)]
    pub variant: Option<VariantRecord>,

    /// Scalar: written only when it differs from the previous regulation.
    #[serde(default)]
    pub stats: Option<Stats>,

    /// Ordered; index 0 is slot 1. Positional ordering makes "slot 2 requires slot 1"
    /// unrepresentable rather than merely validated. See absence note above.
    #[serde(default)]
    pub types: Option<Vec<String>>,

    /// See absence note above.
    #[serde(default)]
    pub abilities: Option<Abilities>,

    /// See absence note above.
    #[serde(default)]
    pub learnset: Option<Vec<LearnsetEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantRecord {
    /// Identifies the base form within the same `national_dex_no`.
    #[serde(default)]
    pub base_form_slug: String,
    /// `variant_kind.name`: 'mega', 'regional', 'primal', 'gigantamax', 'paradox', 'form', 'other'.
    pub kind: String,
    #[serde(default)]
    pub required_item: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub hp: i64,
    pub attack: i64,
    pub defense: i64,
    pub sp_attack: i64,
    pub sp_defense: i64,
    pub speed: i64,
}

impl Stats {
    pub fn total(&self) -> i64 {
        self.hp + self.attack + self.defense + self.sp_attack + self.sp_defense + self.speed
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Abilities {
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default)]
    pub secondary: Option<String>,
    #[serde(default)]
    pub hidden: Option<String>,
}

impl Abilities {
    /// (slot name, ability name) for each occupied slot.
    pub fn occupied(&self) -> Vec<(&'static str, &str)> {
        let mut v = Vec::new();
        if let Some(a) = &self.primary {
            v.push(("primary", a.as_str()));
        }
        if let Some(a) = &self.secondary {
            v.push(("secondary", a.as_str()));
        }
        if let Some(a) = &self.hidden {
            v.push(("hidden", a.as_str()));
        }
        v
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnsetEntry {
    #[serde(rename = "move")]
    pub move_name: String,
    pub method: LearnMethod,
    /// Only meaningful for `level-up`; stored as 0 otherwise.
    #[serde(default)]
    pub level: Option<i64>,
}
