# Frozen agent benchmark

`frozen.json` is the complete pre-run contract. It fixes the source revision,
fixture, prompts, ground truth, seed, tool allowlists, time limit, turn limit,
reset rule, and adoption guards. Each arm must emit one JSON line with the fields
checked by `report.py`: arm, task_id, turns, failed_invocations, correct, outcome,
and elapsed_seconds.

Run `build_fixture.py DIRECTORY` once for each arm and task. It refuses an
existing directory, initializes a Git repository, and makes the history fixture.
Do not change the fixture after creation. `report.py` prints turns, failed calls,
correctness, and failed-run reasons for each arm.
