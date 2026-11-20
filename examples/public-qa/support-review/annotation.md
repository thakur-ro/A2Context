# Passage support review

One AI reviewer read all ten candidate passages and the reference aliases for each question. The packet removes retrieval ranks, original positions, gold markers, detector outputs, and membership in the two test requests. It sorts passages by the SHA-256 digest of their title, a newline, and their text. The reviewer saw the questions in dataset order. No human adjudication took place.

For each question the reviewer recorded sets of passage IDs sufficient to support the reference, sets sufficient to support another reasonable answer, and sets that supplied only indirect or ambiguous evidence. Each inner list is a conjunction. Any complete list counts as a witness. This permits a duration and a separately dated event to support one answer together. Candidates absent from all lists supplied no additional witness in this review.

For each nine-passage request, assign the first applicable class in this order.

1. **Reference supported.** All passages for at least one reference witness are present.
2. **Other answer supported.** The text supports a different answer to the question as worded.
3. **Uncertain.** A partial attribution or ambiguous fact remains.
4. **No support identified.** The reviewer found no sufficient witness in the supplied passages.

The primary detector comparison uses any supported answer for the negative class and no support identified for the positive class. Report uncertain requests separately. Report alternate answers even when both arms remain answerable. The labels describe textual support under this rubric. A model might answer correctly from memory even after every witness is removed.

Run the command below to repeat the reading without exposing the gold markers or detector scores. Keep the generated passage text outside the checkout. The saved review contains digests and the reviewer's own notes.

```sh
python3 examples/public-qa/support-review/review_packet.py /path/to/pinned-qa.jsonl.gz > /tmp/qa-reading-packet.jsonl
```
