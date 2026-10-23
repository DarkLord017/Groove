import re

import numpy as np
from rapidfuzz import fuzz
from sentence_transformers import CrossEncoder, SentenceTransformer

RAPIDFUZZ_THRESHOLD = 85
MIN_RAPIDFUZZ_FLOOR = 40
MIN_MINILM_FLOOR = 0.30
CROSS_ENCODER_THRESHOLD = 0.6
CROSS_ENCODER_THRESHOLD_SAME_TYPE = 0.35
CROSS_ENCODER_MODEL = "cross-encoder/stsb-distilroberta-base"
BI_ENCODER_MODEL = "sentence-transformers/all-MiniLM-L6-v2"


TYPE_PATTERNS = [
    ("email", r"\bemail\b"),
    ("phone", r"\bphone\b"),
    ("website", r"\b(website|url)\b"),
    ("type", r"\btype\b"),
    ("identifier", r"\b(number|id)\b"),
    ("name", r"\bname\b"),
    ("company", r"\b(company|organization|business)\b"),
]

_cross_model = CrossEncoder(CROSS_ENCODER_MODEL)
_bi_model = SentenceTransformer(BI_ENCODER_MODEL)


def _classify(text: str) -> str | None:
    low = text.lower()
    for label, pattern in TYPE_PATTERNS:
        if re.search(pattern, low):
            return label
    return None


def find_match(candidate: str, known_keys: list[str]) -> str | None:
    """Returns the key in known_keys that candidate is the same field as, or None if
    it doesn't match any of them (meaning candidate is a new field).
    """
    if not known_keys:
        return None

    for key in known_keys:
        if fuzz.ratio(candidate, key) >= RAPIDFUZZ_THRESHOLD:
            return key

    all_texts = [candidate] + known_keys
    vecs = _bi_model.encode(all_texts, normalize_embeddings=True)
    candidate_vec = vecs[0]
    minilm_scores = {key: float(np.dot(candidate_vec, vecs[i + 1])) for i, key in enumerate(known_keys)}

    candidate_type = _classify(candidate)
    eligible = []
    for key in known_keys:
        passes_floor = fuzz.ratio(candidate, key) >= MIN_RAPIDFUZZ_FLOOR or minilm_scores[key] >= MIN_MINILM_FLOOR
        if not passes_floor:
            continue
        key_type = _classify(key)
        type_conflict = candidate_type is not None and key_type is not None and candidate_type != key_type
        if type_conflict:
            continue
        eligible.append(key)
    if not eligible:
        return None

    pairs = [(candidate, key) for key in eligible]
    scores = _cross_model.predict(pairs)
    best_idx = max(range(len(scores)), key=lambda i: scores[i])
    best_key = eligible[best_idx]

    same_type_confirmed = candidate_type is not None and _classify(best_key) == candidate_type
    threshold = CROSS_ENCODER_THRESHOLD_SAME_TYPE if same_type_confirmed else CROSS_ENCODER_THRESHOLD
    if scores[best_idx] >= threshold:
        return best_key
    return None
