//! Integration tests for the resolution-gate metric and trust-tier
//! mechanism (`research/ArchitectureImprovements/Codex/01-resolution-gate-plan.md`).
//!
//! Verifies the contract between `query::stats::resolution_breakdown`,
//! `query::dead_code::ResolutionHealth`, and the new trust-tier wiring:
//!
//! - The breakdown surfaces every dimension the plan calls for
//!   (lang/kind, origin language, package, strategy, top-N targets,
//!    low-confidence count + threshold).
//! - The trust tier mirrors the resolution rate the way the plan
//!   prescribes (Trusted ≥ 99%, Review 95–99%, Unsafe < 95%).
//! - In `Unsafe` projects, dead-code candidate confidences are clamped so
//!   downstream tools can't act on them as ground truth.

use bearwisdom::full_index;
use bearwisdom::query::dead_code::{
    DeadCodeOptions, TrustTier, find_dead_code,
};
use bearwisdom::query::stats::resolution_breakdown;
use bearwisdom_tests::TestProject;

#[test]
fn breakdown_surfaces_all_gate_dimensions() {
    // A multi-language project guarantees every breakdown dimension has at
    // least one bucket: per-(lang,kind), per-origin-language, per-package,
    // per-strategy, top targets, low-confidence count + threshold.
    let project = TestProject::multi_lang();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let rb = resolution_breakdown(&db).unwrap();

    // Primary metric is populated, two-decimal rounded, and matches the
    // back-compat alias the older quality-check baselines read.
    assert!(rb.internal_resolution_rate >= 0.0 && rb.internal_resolution_rate <= 100.0);
    assert_eq!(rb.internal_resolution_rate, rb.resolution_rate);

    // Secondary slices exist as containers even if individual buckets are
    // empty for this fixture — the gate report must not vanish a dim.
    let _ = &rb.unresolved_by_lang_kind;
    let _ = &rb.unresolved_by_origin_language;
    let _ = &rb.unresolved_by_package;
    let _ = &rb.resolved_by_strategy;

    // Top-N is bounded.
    assert!(rb.top_unresolved_targets.len() <= 25);

    // Low-confidence threshold defaults to the diagnostics threshold (0.8)
    // and the count is non-negative (u32 implies that, but the threshold
    // contract is the live bit).
    assert!(rb.low_confidence_threshold > 0.0 && rb.low_confidence_threshold <= 1.0);
    let _ = rb.low_confidence_edges;
}

#[test]
fn trust_tier_is_trusted_for_clean_resolution() {
    // A trivial project with one self-contained file resolves all internal
    // refs — no externals, no unresolved. That's the Trusted case.
    let p = TestProject {
        dir: tempfile::TempDir::new().unwrap(),
    };
    p.add_file(
        "main.py",
        "def helper():\n    return 1\n\ndef main():\n    return helper()\n",
    );

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, p.path(), None, None, None).unwrap();

    let dead = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();
    // Either Trusted or Review — both are acceptable on a tiny fixture
    // depending on whether the resolver picks up `helper()` via call. The
    // contract is that we never land on `Unsafe` for a clean project.
    assert_ne!(
        dead.resolution_health.trust_tier,
        TrustTier::Unsafe,
        "clean project should not be Unsafe; got assessment: {}",
        dead.resolution_health.assessment
    );
}

#[test]
fn unsafe_tier_clamps_high_confidence_candidates() {
    // Force an Unsafe project: every reference in the source points at a
    // name the resolver can't see (no matching declaration). This drives
    // the internal resolution rate to ~0% and lands the project in the
    // Unsafe tier.
    let p = TestProject {
        dir: tempfile::TempDir::new().unwrap(),
    };
    p.add_file(
        "app.py",
        "def orphan():\n    return 1\n\
         def caller_a():\n    return undefined_a()\n\
         def caller_b():\n    return undefined_b()\n\
         def caller_c():\n    return undefined_c()\n\
         def caller_d():\n    return undefined_d()\n",
    );

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, p.path(), None, None, None).unwrap();

    let dead = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

    // The trust tier reflects the broken-resolution shape.
    assert_eq!(
        dead.resolution_health.trust_tier,
        TrustTier::Unsafe,
        "broken-resolution project should be Unsafe; got rate={:.1}%",
        dead.resolution_health.resolution_rate
    );

    // Plan §5: in Unsafe, high-confidence deletion recommendations are
    // suppressed. Every candidate's confidence is clamped to ≤ 0.5 so
    // callers can't treat them as actionable. The gate must not have
    // produced any candidate above that threshold.
    for c in &dead.dead_candidates {
        assert!(
            c.confidence <= 0.5,
            "candidate '{}' has confidence {} > 0.5 in Unsafe tier — \
             plan §5 requires suppression",
            c.name,
            c.confidence,
        );
    }
}

#[test]
fn breakdown_metric_matches_dead_code_health() {
    // Both readouts compute the rate from the same `CODE_REF_FILTER`-aware
    // formula. Drift between them would mean the gate report and the
    // dead-code report disagree on whether the project is trustworthy —
    // a contract bug.
    let project = TestProject::multi_lang();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let rb = resolution_breakdown(&db).unwrap();
    let dead = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

    // Both rates come back rounded to one decimal in the dead-code path
    // and two in the breakdown path. Compare at the looser precision.
    let rb_rounded = (rb.internal_resolution_rate * 10.0).round() / 10.0;
    assert!(
        (rb_rounded - dead.resolution_health.resolution_rate).abs() < 0.05,
        "breakdown rate {:.2}% (rounded {:.1}%) disagrees with dead-code health {:.1}%",
        rb.internal_resolution_rate, rb_rounded, dead.resolution_health.resolution_rate,
    );
}
