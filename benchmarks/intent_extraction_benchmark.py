"""Benchmarks site_tree._resolve_intent against realistic full prompts, across
several website categories (travel, shopping, hiring, SaaS signup, hotel booking,
food delivery) - checking two things:

1. SAME_INTENT groups: prompts doing the same underlying task on the same kind of
   site, but with different site/product/city/role/etc. filled in, should all
   resolve to the SAME intent string.
2. DIFFERENT_INTENT groups: genuinely different tasks on the SAME domain should
   resolve to DIFFERENT intent strings (the over-merge failure mode - a domain
   accumulating several real intents must keep them apart).

Writes to throwaway "benchmark-*.test" domains in the real Qdrant site_intents
collection, and deletes them again at the end regardless of outcome.

Run from the learn package's venv (needs its deps - qdrant-client, spacy,
sentence-transformers, rapidfuzz):
    cd learn && uv run python ../benchmarks/intent_extraction_benchmark.py
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "learn" / "src"))

from learn import site_tree  # noqa: E402
from qdrant_client.http import models  # noqa: E402

SAME_INTENT_GROUPS = {
    "flights": [
        "go to airindia.com and look up flights on 25th october mumbai to delhi. i am a student and need a flight of early morning and only direct flights. give me a cheap one. This is a lookup only - do not log in or enter any personal/payment details, just find and report the flight.",
        "go to goindigo.in and look up flights on 20th november bangalore to chennai. i am a student and need a flight of early morning and only direct flights. give me a cheap one. This is a lookup only - do not log in or enter any personal/payment details, just find and report the flight.",
        "go to cleartrip.com and search for flights on 3rd december goa to pune. i need an early morning direct flight, find the cheapest one and report it back - do not log in or enter any payment details.",
    ],
    "shopping": [
        "go to amazon.com and buy a pair of running shoes under $50 in size 10. find the best rated option and add it to the cart, but do not check out or enter any payment details.",
        "go to flipkart.com and buy a wireless mouse under ₹1000. find the best rated option and add it to the cart, but do not check out or enter any payment details.",
        "navigate to ebay.com and buy a used laptop under $300. find the best rated option and add it to the cart, but do not check out or enter any payment details.",
    ],
    "job_application": [
        "go to careers.google.com and fill out a job application for a software engineer role in mountain view. enter my details in the form fields but do not submit the application.",
        "go to jobs.netflix.com and fill out a job application for a product manager role in los gatos. enter my details in the form fields but do not submit the application.",
        "navigate to careers.microsoft.com and fill out a job application for a data scientist role in redmond. enter my details in the form fields but do not submit the application.",
    ],
    "saas_trial_signup": [
        "go to notion.so and sign up for a free trial of their team plan. fill in the signup form but do not enter any real payment details.",
        "go to figma.com and sign up for a free trial of their professional plan. fill in the signup form but do not enter any real payment details.",
        "navigate to slack.com and sign up for a free trial of their business plan. fill in the signup form but do not enter any real payment details.",
    ],
    "hotel_booking": [
        "go to booking.com and look up hotels in goa for 2 nights starting 14th december. i need a 4-star hotel with free breakfast, find the cheapest one and report it - do not book or enter payment details.",
        "go to makemytrip.com and look up hotels in manali for 3 nights starting 5th january. i need a 4-star hotel with free breakfast, find the cheapest one and report it - do not book or enter payment details.",
        "navigate to oyorooms.com and search for hotels in jaipur for 2 nights starting 20th february. i need a 4-star hotel with free breakfast, find the cheapest one and report it - do not book or enter payment details.",
    ],
    "food_delivery": [
        "go to swiggy.com and order biryani from a highly rated restaurant near koramangala. add it to the cart but do not place the order or enter payment details.",
        "go to zomato.com and order pizza from a highly rated restaurant near indiranagar. add it to the cart but do not place the order or enter payment details.",
        "navigate to doordash.com and order sushi from a highly rated restaurant near downtown seattle. add it to the cart but do not place the order or enter payment details.",
    ],
}

DIFFERENT_INTENT_GROUPS = {
    "amazon_mixed_tasks": [
        "go to amazon.com and buy a pair of running shoes under $50 in size 10. find the best rated option and add it to the cart, but do not check out.",
        "go to amazon.com and track my most recent order. report back its delivery status.",
        "go to amazon.com and start a return for an item i bought last week. find the return flow and report what information it asks for, but do not submit the return.",
    ],
    "careers_site_mixed_tasks": [
        "go to careers.google.com and fill out a job application for a software engineer role in mountain view. enter my details but do not submit.",
        "go to careers.google.com and check the status of a job application i already submitted. report back what it says.",
        "go to careers.google.com and look up what benefits are offered for engineering roles. report back a summary.",
    ],
}


def run_same_intent_group(name: str, prompts: list[str]) -> bool:
    domain = f"benchmark-{name}.test"
    resolved = [site_tree._resolve_intent(domain, p) for p in prompts]
    passed = len(set(resolved)) == 1
    print(f"[{'PASS' if passed else 'FAIL'}] same_intent/{name}")
    for p, r in zip(prompts, resolved):
        print(f"    -> {r!r}   (from: {p[:70]}...)")
    return passed


def run_different_intent_group(name: str, prompts: list[str]) -> bool:
    domain = f"benchmark-{name}.test"
    resolved = [site_tree._resolve_intent(domain, p) for p in prompts]
    passed = len(set(resolved)) == len(resolved)
    print(f"[{'PASS' if passed else 'FAIL'}] different_intent/{name}")
    for p, r in zip(prompts, resolved):
        print(f"    -> {r!r}   (from: {p[:70]}...)")
    return passed


def cleanup_benchmark_domains() -> None:
    all_domains = set(SAME_INTENT_GROUPS) | set(DIFFERENT_INTENT_GROUPS)
    for name in all_domains:
        domain = f"benchmark-{name}.test"
        flt = models.Filter(must=[models.FieldCondition(key="domain", match=models.MatchValue(value=domain))])
        for collection in (site_tree.INTENTS_COLLECTION, site_tree.STAGES_COLLECTION):
            site_tree._qdrant.delete(collection_name=collection, points_selector=models.FilterSelector(filter=flt))


def main() -> None:
    results = []
    try:
        for name, prompts in SAME_INTENT_GROUPS.items():
            results.append(run_same_intent_group(name, prompts))
        for name, prompts in DIFFERENT_INTENT_GROUPS.items():
            results.append(run_different_intent_group(name, prompts))
    finally:
        cleanup_benchmark_domains()

    total = len(results)
    passed = sum(results)
    print(f"\n{passed}/{total} groups passed")
    if passed != total:
        sys.exit(1)


if __name__ == "__main__":
    main()
