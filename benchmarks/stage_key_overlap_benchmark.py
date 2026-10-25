
KEY_OVERLAP_THRESHOLD = 0.5

# Real segments collected live from stage_llm.segment_pairs, two task types, two
# runs each (one shopping run and one flight run each failed segmentation entirely -
# a separate, already-noted reliability issue, not relevant to this benchmark).
REAL_SEGMENTS = [
    # flight_search run 1 (mumbai -> delhi)
    {"task": "flight_search", "run": 1, "name": "enter search criteria",
     "keys": {'textbox "origin"', 'textbox "destination"', 'checkbox "non stop"'}},
    {"task": "flight_search", "run": 1, "name": "submit search",
     "keys": {'button "Search"'}},
    # flight_search run 2 (bangalore -> chennai) - segmented into 4 instead of 2
    {"task": "flight_search", "run": 2, "name": "entering origin",
     "keys": {'textbox "origin"'}},
    {"task": "flight_search", "run": 2, "name": "entering destination",
     "keys": {'textbox "destination"'}},
    {"task": "flight_search", "run": 2, "name": "applying filter",
     "keys": {'checkbox "non stop"'}},
    {"task": "flight_search", "run": 2, "name": "submitting search",
     "keys": {'button "Search"'}},
    # shopping run 1 (running shoes)
    {"task": "shopping", "run": 1, "name": "enter search criteria",
     "keys": {'textbox "search"'}},
    {"task": "shopping", "run": 1, "name": "select product",
     "keys": {'link "best rated option"'}},
    {"task": "shopping", "run": 1, "name": "add to cart",
     "keys": {'button "Add to Cart"'}},
]

# (index_a, index_b, expect_match) - ground truth is about whether these are
# genuinely the same real-world stage, independent of whether their names match.
TEST_PAIRS = [
    # Same segmentation shape, different wording -> should match
    (1, 5, True, "submit search (flight r1) vs submitting search (flight r2) - identical keys, different wording"),
    # Genuinely different fields within the same run -> should not match
    (2, 3, False, "entering origin vs entering destination - disjoint fields"),
    (2, 4, False, "entering origin vs applying filter - disjoint fields"),
    (3, 4, False, "entering destination vs applying filter - disjoint fields"),
    # Same NAME text, completely different task/fields -> should NOT match (the case
    # name-based matching would get wrong, since the text is identical)
    (0, 6, False, "'enter search criteria' flight vs 'enter search criteria' shopping - same name, different fields entirely"),
    # Different stages within the same shopping run -> should not match
    (6, 7, False, "enter search criteria vs select product (shopping) - disjoint fields"),
    (7, 8, False, "select product vs add to cart (shopping) - disjoint fields"),
]


def jaccard(a: set, b: set) -> float:
    if not a or not b:
        return 0.0
    return len(a & b) / len(a | b)


def main() -> None:
    results = []
    for i, j, expect_match, label in TEST_PAIRS:
        seg_a, seg_b = REAL_SEGMENTS[i], REAL_SEGMENTS[j]
        score = jaccard(seg_a["keys"], seg_b["keys"])
        got_match = score >= KEY_OVERLAP_THRESHOLD
        passed = got_match == expect_match
        results.append(passed)
        print(f"[{'PASS' if passed else 'FAIL'}] score={score:.2f} expect_match={expect_match} got_match={got_match}")
        print(f"    {label}")
        print(f"    a: {seg_a['name']!r} {seg_a['keys']}")
        print(f"    b: {seg_b['name']!r} {seg_b['keys']}")
        print()

    total, passed = len(results), sum(results)
    print(f"{passed}/{total} pairs correct")


if __name__ == "__main__":
    main()
