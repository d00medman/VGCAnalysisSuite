//! End-to-end verification of the temporal model.
//!
//! Each test migrates its own throwaway schema in a real Postgres, so tests are
//! independent, run in parallel, and need no fixture files. These mirror the phase-7
//! targets in devlog/SchemaAndIngestPlan.md.
//!
//! Needs a running Postgres: `docker compose up -d postgres` from the repo root, or
//! point `POKEDEX_TEST_DATABASE_URL` elsewhere.

use pokedex::model::*;
use pokedex::{ingest, regulation, Db};
use postgres::types::ToSql;
use postgres::{Client, NoTls};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};

const DEFAULT_URL: &str = "postgres://pokedex:pokedex@127.0.0.1:5432/pokedex";

/// A connection whose search_path is a fresh schema, dropped again on `Drop`.
struct TestDb {
    db: Db,
    schema: String,
}

impl TestDb {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let url = std::env::var("POKEDEX_TEST_DATABASE_URL").unwrap_or(DEFAULT_URL.into());
        let mut client = Client::connect(&url, NoTls).unwrap_or_else(|e| {
            panic!("connecting to {url}: {e}\n(start it with `docker compose up -d postgres`)")
        });
        let schema = format!(
            "test_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        );
        client
            .batch_execute(&format!("CREATE SCHEMA {schema}; SET search_path TO {schema}"))
            .unwrap();
        TestDb { db: Db::from_client(client), schema }
    }

    fn migrated() -> Self {
        let mut t = Self::new();
        t.migrate().unwrap();
        t
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = self
            .db
            .client()
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
    }
}

impl Deref for TestDb {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.db
    }
}
impl DerefMut for TestDb {
    fn deref_mut(&mut self) -> &mut Db {
        &mut self.db
    }
}

fn db_with_regs(regs: &[(&str, &str)]) -> TestDb {
    let mut db = TestDb::migrated();
    for (name, from) in regs {
        regulation::add(db.client(), name, from, None).unwrap();
    }
    db
}

fn two_regs() -> TestDb {
    db_with_regs(&[("Reg A", "2023-01-01"), ("Reg B", "2024-01-01")])
}

fn stats(hp: i64, atk: i64, def: i64, spa: i64, spd: i64, spe: i64) -> Stats {
    Stats { hp, attack: atk, defense: def, sp_attack: spa, sp_defense: spd, speed: spe }
}

fn poke(dex: i64, form: &str, name: &str, types: &[&str], st: Stats) -> PokemonRecord {
    PokemonRecord {
        national_dex_no: dex,
        form_slug: form.into(),
        name: name.into(),
        genus: None,
        height_dm: None,
        weight_hg: None,
        variant: None,
        stats: Some(st),
        types: Some(types.iter().map(|s| s.to_string()).collect()),
        abilities: None,
        learnset: None,
    }
}

fn snap(reg: &str, pokemon: Vec<PokemonRecord>) -> Snapshot {
    Snapshot { regulation: reg.into(), abilities: vec![], moves: vec![], pokemon, items: None }
}

fn query_i64(db: &mut Db, sql: &str, params: &[&(dyn ToSql + Sync)]) -> i64 {
    db.client().query_one(sql, params).unwrap().get(0)
}

fn effectiveness(db: &mut Db, dex: i64, atk_type: &str, reg: i64) -> i64 {
    query_i64(
        db,
            "SELECT e.multiplier_pct FROM pokemon_defense_effectiveness e
             JOIN pokemon p ON p.id = e.pokemon_id
             JOIN type t ON t.id = e.attacking_type_id
             WHERE p.national_dex_no = $1 AND p.form_slug = '' AND t.name = $2
               AND e.regulation_id = $3",
        &[&dex, &atk_type, &reg],
    )
}

fn count(db: &mut Db, sql: &str) -> i64 {
    query_i64(db, sql, &[])
}

#[test]
fn seed_data_is_complete() {
    let mut db = TestDb::migrated();
    assert_eq!(count(&mut db, "SELECT count(*) FROM type"), 18);
    assert_eq!(count(&mut db, "SELECT count(*) FROM type_chart_rule"), 324);
    assert_eq!(count(&mut db, "SELECT count(*) FROM nature"), 25);
    assert_eq!(count(&mut db, "SELECT count(*) FROM variant_kind"), 7);
    // Known Gen 6+ distribution.
    assert_eq!(count(&mut db, "SELECT count(*) FROM type_chart_rule WHERE multiplier_pct=0"), 8);
    assert_eq!(count(&mut db, "SELECT count(*) FROM type_chart_rule WHERE multiplier_pct=50"), 61);
    assert_eq!(count(&mut db, "SELECT count(*) FROM type_chart_rule WHERE multiplier_pct=200"), 51);
}

#[test]
fn dual_type_effectiveness_multiplies_exactly() {
    let mut db = two_regs();
    let s = snap("Reg A", vec![
        poke(130, "", "Gyarados", &["water", "flying"], stats(95, 125, 79, 60, 100, 81)),
        poke(6, "", "Charizard", &["fire", "flying"], stats(78, 84, 78, 109, 85, 100)),
    ]);
    ingest::apply(db.client(), &s).unwrap();
    assert_eq!(effectiveness(&mut db, 130, "electric", 1), 400, "2x * 2x");
    assert_eq!(effectiveness(&mut db, 6, "ground", 1), 0, "immunity absorbs the 2x");
    // Water vs Fire/Flying: 2x on Fire, neutral on Flying.
    assert_eq!(effectiveness(&mut db, 6, "water", 1), 200);
    // Grass vs Water/Flying: 2x on Water, 0.5x on Flying -> cancels to neutral.
    assert_eq!(effectiveness(&mut db, 130, "grass", 1), 100);
    // Rock vs Fire/Flying: 2x on both.
    assert_eq!(effectiveness(&mut db, 6, "rock", 1), 400);
    // Electric vs Fire/Flying: neutral on Fire, 2x on Flying.
    assert_eq!(effectiveness(&mut db, 6, "electric", 1), 200);
}

#[test]
fn identical_snapshot_across_regulations_stores_no_extra_rows() {
    let mut db = two_regs();
    let p = poke(6, "", "Charizard", &["fire", "flying"], stats(78, 84, 78, 109, 85, 100));

    ingest::apply(db.client(), &snap("Reg A", vec![p.clone()])).unwrap();
    assert_eq!(count(&mut db, "SELECT count(*) FROM pokemon_stats"), 1);

    let rep = ingest::apply(db.client(), &snap("Reg B", vec![p])).unwrap();
    assert_eq!(count(&mut db, "SELECT count(*) FROM pokemon_stats"), 1, "sparse: nothing changed");
    assert_eq!(rep.stats_written, 0);
    assert!(rep.is_noop());
}

#[test]
fn changed_stat_resolves_per_regulation() {
    let mut db = two_regs();
    let base = poke(130, "", "Gyarados", &["water", "flying"], stats(95, 125, 79, 60, 100, 81));
    let mut faster = base.clone();
    faster.stats = Some(stats(95, 125, 79, 60, 100, 91));

    ingest::apply(db.client(), &snap("Reg A", vec![base])).unwrap();
    ingest::apply(db.client(), &snap("Reg B", vec![faster])).unwrap();
    assert_eq!(count(&mut db, "SELECT count(*) FROM pokemon_stats"), 2);

    let speed = |db: &mut Db, reg: i64| -> i64 {
        query_i64(
            db,
            "SELECT base_speed FROM pokemon_stats_effective e
             JOIN pokemon p ON p.id = e.pokemon_id
             WHERE p.national_dex_no = 130 AND e.regulation_id = $1",
            &[&reg],
        )
    };
    assert_eq!(speed(&mut db, 1), 81);
    assert_eq!(speed(&mut db, 2), 91);
}

#[test]
fn pokemon_introduced_later_is_absent_from_earlier_regulations() {
    let mut db = two_regs();
    let late = poke(999, "", "Latecomer", &["fire"], stats(50, 50, 50, 50, 50, 50));
    ingest::apply(db.client(), &snap("Reg B", vec![late])).unwrap();

    let visible = |db: &mut Db, reg: i64| -> i64 {
        query_i64(
            db,
            "SELECT count(*) FROM pokemon_stats_effective e
             JOIN pokemon p ON p.id = e.pokemon_id
             WHERE p.national_dex_no = 999 AND e.regulation_id = $1",
            &[&reg],
        )
    };
    assert_eq!(visible(&mut db, 1), 0, "must not leak backwards into Reg A");
    assert_eq!(visible(&mut db, 2), 1);
}

#[test]
fn variant_resolves_different_stats_than_its_base() {
    let mut db = two_regs();
    let mut mega = poke(6, "mega-x", "Mega Charizard X", &["fire", "dragon"],
                        stats(78, 130, 111, 130, 85, 100));
    mega.variant = Some(VariantRecord {
        base_form_slug: "".into(),
        kind: "mega".into(),
        required_item: Some("Charizardite X".into()),
    });
    let s = snap("Reg A", vec![
        poke(6, "", "Charizard", &["fire", "flying"], stats(78, 84, 78, 109, 85, 100)),
        mega,
    ]);
    ingest::apply(db.client(), &s).unwrap();

    let bst = |db: &mut Db, form: &str| -> i64 {
        query_i64(
            db,
            "SELECT e.base_stat_total FROM pokemon_stats_effective e
             JOIN pokemon p ON p.id = e.pokemon_id
             WHERE p.national_dex_no = 6 AND p.form_slug = $1 AND e.regulation_id = 1",
            &[&form],
        )
    };
    assert_eq!(bst(&mut db, ""), 534);
    assert_eq!(bst(&mut db, "mega-x"), 634);
    // The mega is Fire/Dragon, so Ground hits it for 2x where the base form is immune.
    assert_eq!(effectiveness(&mut db, 6, "ground", 1), 0);
    assert_eq!(count(&mut db, "SELECT count(*) FROM variant WHERE required_item='Charizardite X'"), 1);
}

#[test]
fn dropping_one_learnset_entry_closes_only_that_interval() {
    let mut db = two_regs();
    let mut p = poke(6, "", "Charizard", &["fire", "flying"], stats(78, 84, 78, 109, 85, 100));
    let entry = |m: &str| LearnsetEntry {
        move_name: m.into(), method: LearnMethod::Tutor, level: None,
    };
    p.learnset = Some(vec![entry("flamethrower"), entry("fly"), entry("swift")]);
    ingest::apply(db.client(), &snap("Reg A", vec![p.clone()])).unwrap();

    p.learnset = Some(vec![entry("flamethrower"), entry("fly")]);
    let rep = ingest::apply(db.client(), &snap("Reg B", vec![p])).unwrap();
    assert_eq!(rep.learnset_closed, 1);
    assert_eq!(rep.learnset_opened, 0, "untouched entries must not be rewritten");

    let open_in = |db: &mut Db, reg: i64| -> i64 {
        query_i64(
            db,
            "SELECT count(*) FROM pokemon_move_effective WHERE regulation_id = $1",
            &[&reg],
        )
    };
    assert_eq!(open_in(&mut db, 1), 3, "all three present in Reg A");
    assert_eq!(open_in(&mut db, 2), 2, "swift gone in Reg B");
    // The two survivors still carry their ORIGINAL Reg A window.
    assert_eq!(
        count(&mut db, "SELECT count(*) FROM pokemon_move WHERE valid_from_regulation_id=1 AND valid_to_regulation_id IS NULL"),
        2
    );
}

#[test]
fn omitted_set_is_preserved_but_explicit_empty_closes() {
    let mut db = two_regs();
    let mut p = poke(6, "", "Charizard", &["fire", "flying"], stats(78, 84, 78, 109, 85, 100));
    p.learnset = Some(vec![LearnsetEntry {
        move_name: "flamethrower".into(), method: LearnMethod::LevelUp, level: Some(46),
    }]);
    ingest::apply(db.client(), &snap("Reg A", vec![p.clone()])).unwrap();

    const OPEN: &str = "SELECT count(*) FROM pokemon_move WHERE valid_to_regulation_id IS NULL";
    assert_eq!(count(&mut db, OPEN), 1);

    // None = not provided. Must NOT be read as "remove everything".
    let mut omitted = p.clone();
    omitted.learnset = None;
    let rep = ingest::apply(db.client(), &snap("Reg B", vec![omitted])).unwrap();
    assert_eq!(rep.learnset_closed, 0);
    assert_eq!(count(&mut db, OPEN), 1, "omitting a field must never destroy data");

    // Some(vec![]) = explicitly empty. Must close.
    let mut emptied = p;
    emptied.learnset = Some(vec![]);
    let rep = ingest::apply(db.client(), &snap("Reg B", vec![emptied])).unwrap();
    assert_eq!(rep.learnset_closed, 1);
    assert_eq!(count(&mut db, OPEN), 0);
}

#[test]
fn retroactive_regulation_relinks_the_window_chain() {
    // Inserted out of chronological order on purpose.
    let mut db = db_with_regs(&[("Reg A", "2023-01-01"), ("Reg C", "2024-01-01")]);
    regulation::add(db.client(), "Reg B", "2023-07-01", None).unwrap();

    let w = regulation::windows(db.client()).unwrap();
    let names: Vec<&str> = w.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["Reg A", "Reg B", "Reg C"], "ordering is date-derived, not id-derived");
    assert_eq!(w[0].effective_to.as_deref(), Some("2023-07-01"), "A now ends at B");
    assert_eq!(w[1].effective_to.as_deref(), Some("2024-01-01"));
    assert_eq!(w[2].effective_to, None, "C is current");
    assert_eq!(w[1].prev_regulation_id, Some(w[0].id));
    assert_eq!(w[1].next_regulation_id, Some(w[2].id));
}

#[test]
fn validation_rejects_malformed_records() {
    let mut db = two_regs();
    let mut dup = poke(1, "", "X", &["fire", "fire"], stats(1, 1, 1, 1, 1, 1));
    assert!(ingest::apply(db.client(), &snap("Reg A", vec![dup.clone()])).is_err());

    dup.types = Some(vec!["fire".into(), "water".into(), "grass".into()]);
    assert!(ingest::apply(db.client(), &snap("Reg A", vec![dup.clone()])).is_err());

    dup.types = Some(vec!["nonsense".into()]);
    assert!(ingest::apply(db.client(), &snap("Reg A", vec![dup.clone()])).is_err());

    dup.types = Some(vec!["fire".into()]);
    dup.abilities = Some(Abilities { primary: None, secondary: Some("blaze".into()), hidden: None });
    assert!(ingest::apply(db.client(), &snap("Reg A", vec![dup])).is_err());

    assert!(ingest::apply(db.client(), &snap("Nonexistent Reg", vec![])).is_err());

    // Every failure rolled back: nothing partial survived.
    assert_eq!(count(&mut db, "SELECT count(*) FROM pokemon"), 0);
}

#[test]
fn variant_without_its_base_in_the_snapshot_is_rejected() {
    let mut db = two_regs();
    let mut orphan = poke(9, "mega", "M", &["water"], stats(1, 1, 1, 1, 1, 1));
    orphan.variant = Some(VariantRecord {
        base_form_slug: "".into(), kind: "mega".into(), required_item: None,
    });
    assert!(ingest::apply(db.client(), &snap("Reg A", vec![orphan])).is_err());
    assert_eq!(count(&mut db, "SELECT count(*) FROM variant"), 0);
}

#[test]
fn schema_rejects_a_second_open_window_on_one_slot() {
    let mut db = two_regs();
    ingest::apply(db.client(), &snap("Reg A", vec![
        poke(6, "", "Charizard", &["fire", "flying"], stats(78, 84, 78, 109, 85, 100)),
    ])).unwrap();

    // Bypass the ingest layer entirely: the guarantee must live in the schema.
    let err = db.client().execute(
        "INSERT INTO pokemon_type (pokemon_id, slot, type_id, valid_from_regulation_id)
         SELECT id, 1, 11, 2 FROM pokemon WHERE national_dex_no = 6",
        &[],
    );
    assert!(err.is_err(), "schema must reject a second open window even on raw SQL");
    let msg = format!("{:?}", err.unwrap_err());
    assert!(msg.contains("pokemon_type_one_open_window"), "{msg}");
}

#[test]
fn migrations_are_idempotent() {
    let mut db = TestDb::migrated();
    assert_eq!(db.schema_version().unwrap(), pokedex::migrate::LATEST_VERSION);
    assert!(db.migrate().unwrap().is_empty(), "re-running must apply nothing");
    db.require_current_schema().unwrap();
}

#[test]
fn older_snapshot_after_a_newer_one_is_refused() {
    let mut db = two_regs();
    let p = poke(6, "", "Charizard", &["fire", "flying"], stats(78, 84, 78, 109, 85, 100));
    ingest::apply(db.client(), &snap("Reg B", vec![p.clone()])).unwrap();
    let err = ingest::apply(db.client(), &snap("Reg A", vec![p.clone()])).unwrap_err();
    assert!(err.to_string().contains("oldest first"), "{err}");
    // Re-applying the newest one is still allowed.
    assert!(ingest::apply(db.client(), &snap("Reg B", vec![p])).unwrap().is_noop());
}

#[test]
fn display_names_are_stored_and_not_blanked_by_bare_references() {
    let mut db = two_regs();
    let mut s = snap("Reg A", vec![]);
    s.moves = vec![MoveRecord {
        name: "kings-shield".into(),
        display_name: Some("King's Shield".into()),
        type_name: "steel".into(),
        damage_class: DamageClass::Status,
        power: None,
        accuracy: None,
        pp: Some(10),
        priority: 4,
        secondary_effect: None,
        effect_chance: None,
    }];
    s.abilities = vec![AbilityRecord {
        name: "flower-veil".into(),
        display_name: Some("Flower Veil".into()),
        description: None,
    }];
    let mut p = poke(670, "", "Floette", &["fairy"], stats(54, 45, 47, 75, 98, 52));
    p.abilities = Some(Abilities { primary: Some("flower-veil".into()), secondary: None, hidden: None });
    p.learnset = Some(vec![LearnsetEntry {
        move_name: "kings-shield".into(), method: LearnMethod::Machine, level: None,
    }]);
    s.pokemon = vec![p];
    ingest::apply(db.client(), &s).unwrap();

    let name = |db: &mut Db, sql: &str| -> String { db.client().query_one(sql, &[]).unwrap().get(0) };
    assert_eq!(name(&mut db, "SELECT display_name FROM move WHERE name = 'kings-shield'"), "King's Shield");
    assert_eq!(name(&mut db, "SELECT display_name FROM ability WHERE name = 'flower-veil'"), "Flower Veil");
}

fn item(name: &str, display: &str, category: ItemCategory) -> ItemRecord {
    ItemRecord {
        name: name.into(),
        display_name: display.into(),
        category,
        fling_power: Some(10),
        description: None,
    }
}

#[test]
fn item_legality_follows_each_regulations_list() {
    let mut db = db_with_regs(&[
        ("Reg A", "2023-01-01"),
        ("Reg B", "2024-01-01"),
        ("Reg C", "2025-01-01"),
    ]);
    let sash = item("focus-sash", "Focus Sash", ItemCategory::Other);
    let sitrus = item("sitrus-berry", "Sitrus Berry", ItemCategory::Berry);
    let with = |reg: &str, items: Vec<ItemRecord>| Snapshot { items: Some(items), ..snap(reg, vec![]) };

    ingest::apply(db.client(), &with("Reg A", vec![sash.clone(), sitrus.clone()])).unwrap();
    let rep = ingest::apply(db.client(), &with("Reg B", vec![sitrus.clone()])).unwrap();
    assert_eq!((rep.items_legal_opened, rep.items_legal_closed), (0, 1), "sash banned in B");
    let rep = ingest::apply(db.client(), &with("Reg C", vec![sash, sitrus])).unwrap();
    assert_eq!((rep.items_legal_opened, rep.items_legal_closed), (1, 0), "sash back in C");

    let legal = |db: &mut Db, reg: i64| -> i64 {
        query_i64(
            db,
            "SELECT count(*) FROM item_legal_effective e JOIN item i ON i.id = e.item_id
             WHERE i.name = 'focus-sash' AND e.regulation_id = $1",
            &[&reg],
        )
    };
    assert_eq!(
        (legal(&mut db, 1), legal(&mut db, 2), legal(&mut db, 3)),
        (1, 0, 1),
        "banned-then-returned needs an interval, not a scalar"
    );

    // items: None must leave legality alone.
    let rep = ingest::apply(db.client(), &snap("Reg C", vec![])).unwrap();
    assert_eq!(rep.items_legal_closed, 0);
    assert_eq!(count(&mut db, "SELECT count(*) FROM item"), 2);
}

#[test]
fn transcript_lines_keep_order_and_one_run_is_active_per_battle() {
    let mut db = TestDb::migrated();
    let c = db.client();
    c.batch_execute(
        "INSERT INTO video (id, name, file, size_bytes) VALUES ('v1', 'a.MP4', 'v1.mp4', 100);
         INSERT INTO battle (video_id) VALUES ('v1');
         INSERT INTO transcript (battle_id, status) SELECT id, 'done' FROM battle;
         INSERT INTO transcript_line (transcript_id, seq, t0, t1, text, conf, clean)
         SELECT t.id, s, s * 2.0, s * 2.0 + 1.5, 'line ' || s, 0.9, true
         FROM transcript t, generate_series(0, 2) s;",
    )
    .unwrap();
    let texts: Vec<String> = c
        .query("SELECT text FROM transcript_line ORDER BY transcript_id, seq", &[])
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(texts, ["line 0", "line 1", "line 2"]);

    // A second run may be queued (the first is done), but not a third while it is active.
    let queue = "INSERT INTO transcript (battle_id) SELECT id FROM battle";
    c.execute(queue, &[]).unwrap();
    let err = c.execute(queue, &[]).unwrap_err();
    assert!(format!("{err:?}").contains("transcript_one_active"), "{err:?}");

    // Deleting the video removes everything under it.
    c.execute("DELETE FROM video", &[]).unwrap();
    assert_eq!(count(&mut db, "SELECT count(*) FROM transcript_line"), 0);
}
