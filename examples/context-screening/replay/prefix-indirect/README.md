# Which calendar matters when the question names an activity?

A visitor asks about either a guided walk among tropical plants or a musicians’ practice before a concert. The calendars list a greenhouse tour and an orchestra rehearsal. Each question identifies an activity by paraphrase. In each of eight matched pairs, the calendar text and order stay fixed while the question changes. Across pairs, Atlas and Birch trade durations, activities and positions. The [protocol](protocol.json) was frozen at `c390e759` before scoring.

For each question, we blanked one duration at a time, keeping its calendar and activity in place. We scored `30` and `60` at the start of the answer and after the supplied text `Activity length: `. A rule selected a calendar only when its edit uniquely reversed which continuation scored higher. The independent outcome was the numeric field in the model’s free answer on the same edit. Free generation received the user’s formatting instruction and started with an empty assistant message. Each original and edited body received one greedy completion from the pinned local Qwen2.5-0.5B-Instruct checkpoint.

## What changed

The supplied-prefix rule selected an edit in 11 of 16 questions. All 11 edits switched the free answer between 30 and 60. In the five abstentions, four questions had no strict reversal and one had two. Seven changed a correct original answer to the other duration. In all four wrong originals the visitor asked about a 60-minute activity and the answer gave 30. Blanking the *other* activity’s duration corrected all four. The hand-written activity lookup found the intended calendar on all 16 questions. Editing that calendar switched seven answers, preserved four numeric fields, and produced five answers of 15 minutes, outside the two registered values.

The table grades each selected answer against the duration listed for the requested activity in the **original** request. Once an edit blanks that duration, an “original value” grade measures preservation of the old fact. The study leaves the appropriate response to a missing duration open. The Rust shadow report also records exact matches against a full answer ending in `minutes`. The numeric grade accepts a resolved value with or without that suffix.

| How to select a source | Selected of 16 | Resolved before and after | Numeric switches | Original correct / wrong | After edit correct / wrong / unresolved |
| --- | ---: | ---: | ---: | ---: | ---: |
| Supplied-prefix reversal | 11 | 11 | 11 | 7 / 4 | 4 / 7 / 0 |
| Empty-prefix reversal | 16 | 12 | 8 | 12 / 4 | 7 / 5 / 4 |
| Hand-written activity lookup | 16 | 11 | 7 | 12 / 4 | 2 / 9 / 5 |
| First entry | 16 | 13 | 9 | 12 / 4 | 8 / 5 / 3 |
| Last entry | 16 | 14 | 2 | 12 / 4 | 10 / 4 / 2 |
| Always Atlas | 16 | 13 | 5 | 12 / 4 | 9 / 4 / 3 |
| Always Birch | 16 | 14 | 6 | 12 / 4 | 9 / 5 / 2 |
| Calendar name in question | 0 | 0 | 0 | — | — |
| Most distinct words shared with question | 0 | 0 | 0 | — | — |

All 48 free generations completed and began with the supplied text. Eleven of the 32 original/edit comparisons switched the numeric field, 16 retained it, and five went outside the candidate set. The supplied-prefix score reversed on all 11 confirmed switches and on one unchanged answer. The empty-prefix score reversed on eight confirmed switches and four unchanged answers, missing three switches. Both counts keep the five outside-set answers separate. The full summary includes outcomes for each source, selector, and arm.

Only **one of the eight matched question pairs** selected different calendars with both numeric switches confirmed. Two pairs selected the same calendar twice. In the remaining five, the rule abstained on one question. In `birch-calendar-30-music-plants`, the answer changed from 60 to 30 when the greenhouse tour’s Birch duration was blanked. With the same calendar text in `birch-calendar-30-music-music`, the answer changed from 30 to 60 when the rehearsal’s Atlas duration was blanked.

Two failure cases help interpret these measurements. In `atlas-calendar-30-plants-music`, the question asked about a 60-minute rehearsal, but the original answer gave the tour’s 30 minutes. Removing the tour duration corrected the answer to 60. In `atlas-calendar-60-plants-plants`, both edits reversed the supplied-prefix preference. The free answer stayed at 60 after blanking Birch and went to 15 after blanking Atlas. The unique-reversal rule abstained.

These measurements cover two paraphrases, one synthetic template and one small checkpoint with the requested output prefix. They measure the effects of specified edits on a fixed continuation and an answer field. Generalization to other paraphrases and claims about internal model mechanisms would require separate experiments.

## Reproduce

From the repository root, verify the saved envelopes and print the counts with the Rust binary and Python’s standard library.

```sh
cargo build --locked
study=examples/context-screening/replay/prefix-indirect
python3 "$study/study.py"
```

To repeat inference, use the local checkpoint and CPU dependency versions in the protocol. Activate that Python environment so the scorer and generator use it. Choose a new output path for each run.

```sh
python3 "$study/study.py" --model "$CHECKPOINT_DIR" --cases /tmp/indirect-repeat.json
cmp "$study/cases.json" /tmp/indirect-repeat.json
```

Two complete local inference passes produced byte-identical case files (SHA-256 `8adcd4da87db116aadbb747827a44fabfd174b9adfef607b36b832ebdc3faa43`) and identical verified summaries. Each row of [cases.json](cases.json) contains the intended source, question and source assignments, empty-prefix log probabilities, and a request-bound `envelope` with the supplied-prefix scores and all three generated trials. To inspect an individual envelope, pass it to `a2context inspect-pending -`.
