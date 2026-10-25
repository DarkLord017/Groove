import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "learn" / "src"))

from learn import key_match  # noqa: E402

DOMAIN = "testbench.com"

# Real, as induced live by awm.induce_workflow() this session - not invented.
STORED_WORKFLOWS = {
    "flight": "Search for a flight between two cities with a non-stop option",
    "cart": "Add a product to the cart",
}

FLOOR = 0.30  # same floor proposed for recall's match_intent / match_workflow
MARGIN = 0.1  # same MIN_MARGIN site_tree.py's select_stage already uses

# Realistic PROMPTS a user would actually type - not reworded descriptions, not the
# stored text itself. Each entry: (prompt, expected_winner_key).
PROMPTS = [
    ("find me a cheap flight from mumbai to delhi", "flight"),
    ("book a nonstop flight between two cities", "flight"),
    ("I need to search flights for my trip", "flight"),
    ("add running shoes to my cart", "cart"),
    ("buy this product", "cart"),
    ("put an item in the shopping cart", "cart"),
    # Genuinely unrelated to either stored workflow - should clear neither confidently.
    ("what's the weather like today", None),
    ("explain how linked lists work", None),
]


def embed(text: str):
    return key_match._bi_model.encode(text, normalize_embeddings=True)


def main() -> None:
    import numpy as np

    stored_vecs = {key: embed(f"{DOMAIN}: {desc}") for key, desc in STORED_WORKFLOWS.items()}

    results = []
    for prompt, expected in PROMPTS:
        query_vec = embed(f"{DOMAIN}: {prompt}")
        scores = {key: float(np.dot(query_vec, v)) for key, v in stored_vecs.items()}
        ranked = sorted(scores.items(), key=lambda kv: kv[1], reverse=True)
        top_key, top_score = ranked[0]
        runner_up_score = ranked[1][1] if len(ranked) > 1 else 0.0

        margin = top_score - runner_up_score
        clears = top_score >= FLOOR and margin >= MARGIN
        if expected is None:
            passed = not clears
        else:
            passed = top_key == expected and clears

        results.append(passed)
        print(f"[{'PASS' if passed else 'FAIL'}] {prompt!r}")
        print(f"    expected={expected!r} top={top_key!r} score={top_score:.3f} runner_up={runner_up_score:.3f} margin={top_score - runner_up_score:.3f}")

    total, passed = len(results), sum(results)
    print(f"\n{passed}/{total} prompts correct")


if __name__ == "__main__":
    main()
