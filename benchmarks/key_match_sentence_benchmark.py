"""
Checks two things per group of realistic candidate phrasings:
1. SAME_CONCEPT groups: different ways of wording the same stage/workflow should
   all match each other (first phrasing becomes the "known" one; every later
   phrasing in the group should resolve to it via find_match).
2. DIFFERENT_CONCEPT groups: genuinely different stages/workflows should NOT match
   each other (the over-merge failure mode).
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "learn" / "src"))

from learn import key_match  # noqa: E402

STAGE_NAME_SAME_CONCEPT = {
    "enter_criteria": [
        "enter search criteria",
        "fill in search fields",
        "input flight details",
        "enter origin and destination",
    ],
    "apply_filter": [
        "apply filter",
        "narrow down results",
        "set filter options",
        "toggle non-stop filter",
    ],
    "submit_search": [
        "submit search",
        "click search button",
        "confirm search",
        "trigger the search",
    ],
    "pick_result": [
        "pick fare option",
        "select a flight",
        "choose fare type",
        "pick the cheapest flight",
    ],
}

STAGE_NAME_DIFFERENT_CONCEPT = [
    "enter search criteria",
    "apply filter",
    "submit search",
    "pick fare option",
    "confirm payment details",
]

WORKFLOW_DESC_SAME_CONCEPT = {
    "search_flights": [
        "Search for cheap flights between two cities",
        "Find a cheap flight between two cities",
        "Look up affordable flights between cities",
        "Search and compare flight prices between two cities",
    ],
    "book_hotel": [
        "Book a hotel room",
        "Reserve a hotel",
        "Find and book accommodation",
        "Search for and reserve a hotel room",
    ],
    "add_to_cart": [
        "Add a product to cart",
        "Put an item in the shopping cart",
        "Select a product and add it to the cart",
        "Add the selected item to your shopping cart",
    ],
}

WORKFLOW_DESC_DIFFERENT_CONCEPT = [
    "Search for cheap flights between two cities",
    "Book a hotel room",
    "Add a product to cart",
    "Check in for a flight",
    "Fill out a job application",
]


def run_same_concept_group(kind: str, label: str, phrasings: list[str]) -> bool:
    known: list[str] = [phrasings[0]]
    results = [phrasings[0]]
    for phrase in phrasings[1:]:
        match = key_match.find_match(phrase, known)
        resolved = match or phrase
        results.append(resolved)
        known.append(resolved)

    passed = len(set(results)) == 1
    print(f"[{'PASS' if passed else 'FAIL'}] {kind}/same_concept/{label}")
    for phrase, resolved in zip(phrasings, results):
        print(f"    {phrase!r:55} -> {resolved!r}")
    return passed


def run_different_concept_group(kind: str, phrasings: list[str]) -> bool:
    known: list[str] = [phrasings[0]]
    results = [phrasings[0]]
    for phrase in phrasings[1:]:
        match = key_match.find_match(phrase, known)
        resolved = match or phrase
        results.append(resolved)
        known.append(resolved)

    passed = len(set(results)) == len(results)
    print(f"[{'PASS' if passed else 'FAIL'}] {kind}/different_concept")
    for phrase, resolved in zip(phrasings, results):
        print(f"    {phrase!r:55} -> {resolved!r}")
    return passed


def main() -> None:
    results = []
    for label, phrasings in STAGE_NAME_SAME_CONCEPT.items():
        results.append(run_same_concept_group("stage_name", label, phrasings))
    results.append(run_different_concept_group("stage_name", STAGE_NAME_DIFFERENT_CONCEPT))

    for label, phrasings in WORKFLOW_DESC_SAME_CONCEPT.items():
        results.append(run_same_concept_group("workflow_desc", label, phrasings))
    results.append(run_different_concept_group("workflow_desc", WORKFLOW_DESC_DIFFERENT_CONCEPT))

    total = len(results)
    passed = sum(results)
    print(f"\n{passed}/{total} groups passed")
    if passed != total:
        sys.exit(1)


if __name__ == "__main__":
    main()
