#!/usr/bin/env python3
"""One load-generator worker for load_bench.py. Not run directly -- it is the
`multiprocessing` (spawn) target the ramp host starts one of per unit of
concurrency.

Because the start method is "spawn", this is a genuinely fresh CPython
interpreter: it does its own `import ODRL_Evaluator`, builds its own rdflib
graphs and its own ODRL 2.2 ontology parse, and shares no evaluator state with
its siblings or with the host. That is deliberate -- it is what makes a worker
comparable to "one more concurrent user" rather than to a thread contending for
one GIL.

Protocol over a duplex Pipe, one line-equivalent per message:
  child -> host  {"ready": True}                    after its own warmup
  host  -> child {"cmd": "go", "duration": s, "offset": k}
  child -> host  {"records": [[slug, ms, end_rel_s, ok], ...]}
  host  -> child {"cmd": "stop"}
Between a "records" reply and the next "go" the worker blocks in recv(): it
holds its memory and burns no CPU, which is what lets the host spawn the pool
once and still time a c=1 step honestly.
"""
import os
import time

import perf_corpus as pc

WARMUP = 5


def worker_main(conn, offset, isolate=True, warmup=WARMUP):
    engine = pc.attach()
    cases = pc.load_cases()
    for i in range(warmup):
        pc.evaluate(engine, cases[(offset + i) % len(cases)], isolate=isolate)
    conn.send({"ready": True, "pid": os.getpid(), "cases": len(cases)})

    idx = offset
    while True:
        msg = conn.recv()
        if msg.get("cmd") == "stop":
            break
        if msg.get("cmd") != "go":
            continue
        t_start = time.perf_counter()
        deadline = t_start + msg["duration"]
        records = []
        while time.perf_counter() < deadline:
            c = cases[idx % len(cases)]
            idx += 1
            ms, dec, err = pc.evaluate(engine, c, isolate=isolate)
            records.append([c["slug"], round(ms, 3),
                            round(time.perf_counter() - t_start, 4),
                            (err is None and dec == c["expected"]), err])
        conn.send({"records": records})
    conn.close()
