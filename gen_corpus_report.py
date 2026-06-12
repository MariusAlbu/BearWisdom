#!/usr/bin/env python3
"""Generate CORPUS-2026-06-11.md from the post-recapture baseline-all.json.

Diffs against the pre-wave snapshot (baseline-all.PRE-WAVE-2026-06-11.json) for
movers and assertion-floor regressions. No source edits; read-only over JSON.
"""
import json, sys, os, sqlite3
from collections import defaultdict

POST = "baseline-all.json"
PRE  = "baseline-all.PRE-WAVE-2026-06-11.json"
OUT  = "CORPUS-2026-06-11.md"
PRIOR_APP_HEADLINE = 86.24  # prior application-class corpus rate

# Exact filter fragments mirrored from query/stats.rs so per-language DB
# queries reproduce the engine's rate_by_language numerator/denominator.
CODE_REF_FILTER = ("u.from_snippet = 0 "
                   "AND NOT (f.language IN ('markdown','mdx') AND u.kind = 'imports')")
GEN_FILTER = ("NOT (f.language = 'dart' "
              "AND (f.path LIKE '%.g.dart' "
              "OR f.path LIKE '%.freezed.dart' "
              "OR f.path LIKE '%/generated/%' "
              "OR f.path LIKE 'generated/%'))")

EDGES_BY_LANG_SQL = f"""
SELECT COALESCE(s.origin_language, f.language) AS lang, COUNT(*)
FROM edges e
JOIN symbols s ON s.id = e.source_id
JOIN files   f ON f.id = s.file_id
WHERE f.origin = 'internal' AND {GEN_FILTER}
GROUP BY lang
"""
UNRES_BY_LANG_SQL = f"""
SELECT COALESCE(s.origin_language, f.language) AS lang, COUNT(*)
FROM unresolved_refs u
JOIN symbols s ON s.id = u.source_id
JOIN files   f ON f.id = s.file_id
WHERE f.origin = 'internal' AND {CODE_REF_FILTER} AND {GEN_FILTER}
GROUP BY lang
"""

def query_db_by_lang(db_path):
    """Return (edges_by_lang, unres_by_lang) from a project's index.db, or (None,None)."""
    if not os.path.isfile(db_path):
        return None, None
    try:
        con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True, timeout=30)
        ed = {l: c for l, c in con.execute(EDGES_BY_LANG_SQL)}
        un = {l: c for l, c in con.execute(UNRES_BY_LANG_SQL)}
        con.close()
        return ed, un
    except Exception as e:
        return ("ERR:" + repr(e)), None

def load(p):
    with open(p, encoding="utf-8") as f:
        return json.load(f)

def corpus_group_key(cc):
    # mirrors corpus_group_key in main.rs
    if cc is None or cc == "":
        return "application"
    if cc.startswith("duplicate-of:"):
        return None
    return cc

def rate(edges, unres):
    denom = edges + unres
    return 100.0 if denom == 0 else round(edges * 100.0 / denom, 2)

def main():
    post = load(POST)
    pre  = load(PRE)
    pprojs = {e["project"]: e for e in post["projects"]}
    qprojs = {e["project"]: e for e in pre["projects"]}

    # ---- corpus_class pooled rates (mirrors corpus_class_report) ----
    groups = defaultdict(lambda: [0, 0])  # group -> [edges, unres]
    all_edges = all_unres = 0
    for e in post["projects"]:
        ed = int(e.get("internal_edges", 0))
        un = int(e.get("internal_unresolved", 0))
        all_edges += ed
        all_unres += un
        g = corpus_group_key(e.get("corpus_class"))
        if g is not None:
            groups[g][0] += ed
            groups[g][1] += un
    app_rate = rate(*groups["application"]) if "application" in groups else 0.0
    all_rate = rate(all_edges, all_unres)

    # ---- per-language aggregate across application-class projects only ----
    # Primary: query each project's post-recapture index.db for the same
    # internal_edges_by_lang / unresolved_by_lang the engine computes, then pool.
    # Fallback for projects whose DB is mid-rebuild (edges=0 but JSON has valid
    # rate_by_language + unresolved_by_lang_kind): reconstruct edges from JSON.
    lang_unres = defaultdict(int)
    lang_edges = defaultdict(int)
    db_errors = []
    missing_dbs = []
    json_fallback_projects = []
    for e in post["projects"]:
        if corpus_group_key(e.get("corpus_class")) != "application":
            continue
        name = e["project"]
        db_path = os.path.join(e["path"], ".bearwisdom", "index.db")
        ed, un = query_db_by_lang(db_path)
        # Fall back to JSON if DB is unavailable or has zero edges but JSON is good
        use_json_fallback = False
        if ed is None or isinstance(ed, str):
            use_json_fallback = True
            if ed is None:
                missing_dbs.append(name)
            else:
                db_errors.append((name, ed))
        else:
            db_total = sum(ed.values())
            json_total = int(e.get("internal_edges", 0))
            # If DB edge count is < 85% of JSON baseline value (mid-rebuild state),
            # fall back to JSON to avoid polluting the language aggregate.
            if json_total > 0 and db_total < json_total * 0.85:
                use_json_fallback = True
                db_errors.append((name, f"DB edges {db_total} < 85% of JSON {json_total}; using JSON fallback"))
        if use_json_fallback:
            json_fallback_projects.append(name)
            # Reconstruct from JSON: unresolved from unresolved_by_lang_kind,
            # edges from rate_by_language. rate=100 langs get no unresolved; edges
            # derived as unres * rate / (100 - rate); rate=100 langs get
            # json internal_edges proportional share (best available approximation).
            rbl = e.get("rate_by_language") or {}
            ubk = e.get("unresolved_by_lang_kind") or {}
            per_lang_unres = defaultdict(int)
            for k, v in ubk.items():
                lang = k.split(".", 1)[0]
                per_lang_unres[lang] += int(v)
            for lang, r in rbl.items():
                r = float(r)
                un_count = per_lang_unres.get(lang, 0)
                lang_unres[lang] += un_count
                if r < 100.0 and un_count > 0:
                    ed_est = int(round(un_count * r / (100.0 - r)))
                    lang_edges[lang] += ed_est
                # r==100.0: unresolved=0, edges unknown without DB; omit from lang_edges
                # so these langs get 100% (denom=0 -> rate=100) in the pooled table.
        else:
            for lang, c in ed.items():
                lang_edges[lang] += c
            for lang, c in un.items():
                lang_unres[lang] += c

    langs = sorted(set(list(lang_unres.keys()) + list(lang_edges.keys())))
    lang_rows = []
    for lang in langs:
        un = lang_unres.get(lang, 0)
        ed = lang_edges.get(lang, 0)
        r = rate(ed, un)
        lang_rows.append((lang, r, ed, un, ""))
    lang_rows.sort(key=lambda x: (-x[3], x[0]))  # unresolved desc, then name

    # ---- movers vs pre-wave (application + all; compare resolution_rate) ----
    movers = []
    for name, pe in pprojs.items():
        qe = qprojs.get(name)
        if qe is None:
            continue
        # skip ghost/skipped: if both edges identical AND unres identical, treat as unchanged
        pr = float(pe.get("resolution_rate", 0.0))
        qr = float(qe.get("resolution_rate", 0.0))
        delta = round(pr - qr, 2)
        movers.append((name, qr, pr, delta,
                       int(qe.get("internal_edges", 0)), int(pe.get("internal_edges", 0)),
                       int(qe.get("internal_unresolved", 0)), int(pe.get("internal_unresolved", 0)),
                       pe.get("corpus_class")))
    ups = sorted([m for m in movers if m[3] > 0], key=lambda x: -x[3])[:20]
    downs = sorted([m for m in movers if m[3] < -1.0], key=lambda x: x[3])

    # ---- assertion-floor regressions: post rate < pre-wave min_resolution_rate ----
    regressions = []
    for name, pe in pprojs.items():
        qe = qprojs.get(name)
        floor = None
        if qe:
            floor = (qe.get("assertions") or {}).get("min_resolution_rate")
        if floor is None:
            continue
        post_rate = float(pe.get("resolution_rate", 0.0))
        if post_rate < float(floor):
            regressions.append((name, float(floor), post_rate,
                                int(pe.get("internal_edges", 0)),
                                int(pe.get("internal_unresolved", 0)),
                                pe.get("corpus_class")))
    regressions.sort(key=lambda x: x[2] - x[1])  # worst breach first

    # ---- generated_excluded totals where nonzero ----
    gen_excl = [(e["project"], int(e["generated_excluded"]))
                for e in post["projects"]
                if int(e.get("generated_excluded", 0)) > 0]
    gen_excl.sort(key=lambda x: -x[1])

    # ---- emit markdown ----
    L = []
    w = L.append
    w(f"# Corpus recapture — 2026-06-11\n")
    w(f"Post-wave honest baseline. Source: `baseline-all.json` (recaptured), "
      f"diffed against `baseline-all.PRE-WAVE-2026-06-11.json`.\n")
    w(f"Captured: {post.get('captured_at')}  |  Projects: {len(post['projects'])}\n")

    # 1. Headline
    w("\n## 1. Corpus headline\n")
    aedges, aunres = groups.get("application", [0, 0])
    w(f"- **Application-class internal resolution rate: {app_rate:.2f}%** "
      f"({aedges} edges / {aunres} unresolved) — prior {PRIOR_APP_HEADLINE:.2f}%, "
      f"delta {app_rate - PRIOR_APP_HEADLINE:+.2f} pts.")
    w(f"- All-projects (every entry, all classes pooled): **{all_rate:.2f}%** "
      f"({all_edges} edges / {all_unres} unresolved).\n")

    # 2. Per-language table
    w("\n## 2. Per-language corpus table (application-class, by unresolved desc)\n")
    w("First per-language-truthful corpus aggregate. Edges and unresolved are "
      "pooled across application-class projects by querying each post-recapture "
      "`index.db` with the engine's own `internal_edges_by_lang` / "
      "`unresolved_by_lang` SQL (COALESCE(origin_language, file_language) "
      "attribution, generated-file + code-ref filters applied). Rate = "
      "edges / (edges + unresolved) * 100.\n")
    if json_fallback_projects:
        w(f"> Note: {len(json_fallback_projects)} project(s) used JSON `rate_by_language` + "
          f"`unresolved_by_lang_kind` fallback (DB mid-rebuild or unavailable): "
          f"{', '.join(json_fallback_projects)}. "
          f"For rate=100 languages in fallback projects, edges are omitted (rate stays 100%).\n")
    if missing_dbs and not json_fallback_projects:
        w(f"> Note: {len(missing_dbs)} application project(s) had no DB: "
          f"{', '.join(missing_dbs)}.\n")
    w("| Language | Rate % | Edges | Unresolved | >=99? |")
    w("|---|---:|---:|---:|---|")
    for lang, r, ed, un, note in lang_rows:
        flag = "OK >=99" if r >= 99.0 else f"GAP {99.0 - r:.2f}"
        w(f"| {lang} | {r:.2f} | {ed}{note} | {un} | {flag} |")
    ge99 = [x for x in lang_rows if x[1] >= 99.0]
    lt99 = [x for x in lang_rows if x[1] < 99.0]
    w(f"\n- Languages >=99: {len(ge99)} — {', '.join(x[0] for x in ge99)}")
    gap_strs = ["{} ({:.2f})".format(x[0], 99.0 - x[1]) for x in lt99]
    w(f"- Languages <99 (gap): {len(lt99)} — " + ", ".join(gap_strs))
    w("")

    # 3. Movers
    w("\n## 3. Biggest per-project movers vs pre-wave\n")
    w("### Top 20 up (resolution_rate)\n")
    w("| Project | pre% | post% | delta | edges pre->post | class |")
    w("|---|---:|---:|---:|---|---|")
    for name, qr, pr, d, qed, ped, qun, pun, cc in ups:
        w(f"| {name} | {qr:.2f} | {pr:.2f} | +{d:.2f} | {qed}->{ped} | {cc or 'application'} |")
    w("\n### Down >1 pt (with cause hypothesis)\n")
    if not downs:
        w("None. No application/framework project regressed more than 1 pt.")
    else:
        w("| Project | pre% | post% | delta | edges pre->post | unres pre->post | class | hypothesis |")
        w("|---|---:|---:|---:|---|---|---|---|")
        # Per-project cause hypotheses derived from unresolved_by_lang_kind diffs.
        KNOWN_CAUSES = {
            "matlab-platemo":   "50K new matlab.calls — MATLAB stdlib-fn extraction now counts attempts",
            "fsharp-saturn":    "641 new fsharp.calls — F# call extraction widened, unresolved surfaced",
            "matlab-prmlt":     "1487 new matlab.calls — same MATLAB extraction widening as matlab-platemo",
            "matlab-exportfig": "1513 new matlab.calls — same MATLAB extraction widening",
            "lua-koreader":     "edges collapsed 162K->14K (vendor/external purge); rate dropped on smaller denominator",
            "fsharp-ionide":    "458 new fsharp.calls — F# call extraction widened",
            "clojure-babashka": "edges collapsed 40K->12K (vendor/external purge); raw rate change on smaller denominator",
            "lua-luals":        "edges collapsed 262K->9K (vendor/external purge); rate dropped on smaller denominator",
            "pascal-heidisql":  "2331 new pascal.calls — Pascal call extraction widened or vendor exclusion removed",
        }
        for name, qr, pr, d, qed, ped, qun, pun, cc in downs:
            hyp = KNOWN_CAUSES.get(name)
            if hyp is None:
                if ped < qed and pun > qun:
                    hyp = "edges dropped + unresolved rose — extractor/resolver regression"
                elif pun > qun and ped >= qed:
                    hyp = "new unresolved surfaced (wider extraction now counts attempts)"
                elif ped > qed and pun > qun:
                    hyp = "more refs extracted; unresolved grew faster than resolved"
                else:
                    hyp = "rate moved on small denominator — count jitter"
            w(f"| {name} | {qr:.2f} | {pr:.2f} | {d:.2f} | {qed}->{ped} | {qun}->{pun} | {cc or 'application'} | {hyp} |")
    w("")

    # 4. corpus_class breakdown
    w("\n## 4. corpus_class breakdown\n")
    w("Duplicates (`duplicate-of:*`) contribute to no group and are excluded.\n")
    w("| Class | Rate % | Edges | Unresolved | Projects |")
    w("|---|---:|---:|---:|---:|")
    cc_counts = defaultdict(int)
    for e in post["projects"]:
        g = corpus_group_key(e.get("corpus_class"))
        if g is not None:
            cc_counts[g] += 1
    for g in sorted(groups.keys()):
        ed, un = groups[g]
        w(f"| {g} | {rate(ed, un):.2f} | {ed} | {un} | {cc_counts[g]} |")
    dups = [e["project"] for e in post["projects"]
            if (e.get("corpus_class") or "").startswith("duplicate-of:")]
    w(f"\n- Excluded duplicates ({len(dups)}): {', '.join(dups) if dups else 'none'}")
    w("")

    # 5. Assertion failures
    w("\n## 5. Assertion-floor regressions (min_resolution_rate tripwires)\n")
    w("Post-recapture resolution_rate measured against the **pre-wave** "
      "`min_resolution_rate` floor (the recapture rewrites floors, so this is "
      "computed from the pre-wave snapshot — these are the real tripwires).\n")
    if not regressions:
        w("**None.** Every project meets or exceeds its pre-wave min_resolution_rate floor.")
    else:
        w("| Project | floor | post% | breach | edges | unresolved | class |")
        w("|---|---:|---:|---:|---:|---:|---|")
        for name, fl, pr, ed, un, cc in regressions:
            w(f"| {name} | {fl:.0f} | {pr:.2f} | {pr - fl:+.2f} | {ed} | {un} | {cc or 'application'} |")
    w("")

    # 6. generated_excluded
    w("\n## 6. generated_excluded totals (nonzero)\n")
    if not gen_excl:
        w("None. No project reported generated_excluded > 0.")
    else:
        w("| Project | generated_excluded |")
        w("|---|---:|")
        for name, n in gen_excl:
            w(f"| {name} | {n} |")
        w(f"\nTotal generated refs excluded: {sum(n for _, n in gen_excl)}")
    w("")

    text = "\n".join(L)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(text)

    # stdout summary for the orchestrator
    print(f"WROTE {OUT}")
    print(f"HEADLINE app={app_rate:.2f}% (prior {PRIOR_APP_HEADLINE}) all={all_rate:.2f}%")
    print(f"langs >=99: {len(ge99)} / <99: {len(lt99)}")
    print(f"movers up: {len(ups)}  down>1pt: {len(downs)}")
    print(f"assertion regressions: {len(regressions)}")
    print(f"generated_excluded projects: {len(gen_excl)}")

if __name__ == "__main__":
    main()
