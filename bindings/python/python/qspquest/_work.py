"""Bounded ordered dispatch for independent analysis work units."""
from collections import deque
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager

from ._analysis import integer


@contextmanager
def ordered_work(items, execute, workers, control=None):
    """Yield input-ordered results with at most workers submitted work units.

    The owner commits checkpoints between yields. On cancellation or failure,
    queued work is cancelled and running units drain before final accounting.
    No progress callback or checkpoint writer runs on a worker thread.
    """
    integer(workers, "workers", 1, 8)
    items = iter(items)
    if workers == 1:
        def serial():
            for item in items:
                if control is not None:
                    control.check()
                yield execute(item)
        yield serial()
        return
    pool = ThreadPoolExecutor(max_workers=workers, thread_name_prefix="qspquest-analysis")
    pending = deque()

    def fill():
        while len(pending) < workers:
            if control is not None:
                control.check()
            try:
                item = next(items)
            except StopIteration:
                break
            pending.append(pool.submit(execute, item))

    def ordered():
        fill()
        while pending:
            if control is not None:
                control.check()
            result = pending.popleft().result()
            yield result
            fill()

    try:
        yield ordered()
    finally:
        for future in pending:
            future.cancel()
        pool.shutdown(wait=True, cancel_futures=True)
