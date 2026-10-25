"""
Checks whether embedding similarity can tell "this stage's post-condition is now true"
from the live page - the diagram's "Redis - done stages, page matches 'after' -> mark
done" box. Nothing in learn/ does this today: site_tree.py's select_stage only compares
condition SENTENCES to other condition sentences (both written by stage_llm, same
register). This instead compares a raw page-text dump against a short natural-language
condition sentence - a real register mismatch worth checking before relying on it.

Both page-text snapshots are real: captured live via claude-in-chrome's get_page_text
against a local test page (search form -> click search -> results appear), not invented.
(A first attempt used read_page instead - its "all" filter output was IDENTICAL before and
after the search, because a hidden <div style="display:none"> still renders into the
accessibility tree. get_page_text correctly excludes hidden content, so it's the one
tested here.)
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "learn" / "src"))

from learn import key_match  # noqa: E402

# Real, from get_page_text on http://localhost:8745/flow_test.html.
BEFORE_SEARCH = """Flight Search
Origin Destination Search"""

AFTER_SEARCH = """Flight Search
Origin Destination Search
Search Results
Flight AI101 - Mumbai to Delhi - $120
Flight AI202 - Mumbai to Delhi - $145
Flight AI303 - Mumbai to Delhi - $98"""

POST_CONDITION = "Search results are shown"
UNRELATED_CONDITION = "User is logged into their account"

FLOOR = 0.30
MARGIN = 0.1


def embed(text: str):
    return key_match._bi_model.encode(text, normalize_embeddings=True)


def main() -> None:
    import numpy as np

    cond_vec = embed(POST_CONDITION)
    unrelated_vec = embed(UNRELATED_CONDITION)
    before_vec = embed(BEFORE_SEARCH)
    after_vec = embed(AFTER_SEARCH)

    cases = [
        ("before-search page vs its own post_condition (should NOT match - not done yet)",
         before_vec, cond_vec, False),
        ("after-search page vs its own post_condition (SHOULD match - stage is done)",
         after_vec, cond_vec, True),
        ("after-search page vs an unrelated condition (should NOT match)",
         after_vec, unrelated_vec, False),
    ]

    results = []
    for label, page_vec, cond_vec_i, expect_match in cases:
        score = float(np.dot(page_vec, cond_vec_i))
        got_match = score >= FLOOR
        passed = got_match == expect_match
        results.append(passed)
        print(f"[{'PASS' if passed else 'FAIL'}] score={score:.3f} expect_match={expect_match} got_match={got_match}")
        print(f"    {label}")

    # Also check the margin between before/after scores against the SAME condition -
    # not just whether each clears the floor in isolation.
    before_score = float(np.dot(before_vec, cond_vec))
    after_score = float(np.dot(after_vec, cond_vec))
    margin = after_score - before_score
    margin_ok = margin >= MARGIN
    results.append(margin_ok)
    print(f"[{'PASS' if margin_ok else 'FAIL'}] before={before_score:.3f} after={after_score:.3f} margin={margin:.3f} (need >= {MARGIN})")
    print("    after-search score clears before-search score by a real margin, not noise")

    total, passed = len(results), sum(results)
    print(f"\n{passed}/{total} checks passed")


if __name__ == "__main__":
    main()
