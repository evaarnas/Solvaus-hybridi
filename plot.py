import csv
import time

import matplotlib.pyplot as plt


def format_duration(seconds):
    total_milliseconds = round(seconds * 1000)
    hours, remainder = divmod(total_milliseconds, 3_600_000)
    minutes, remainder = divmod(remainder, 60_000)
    whole_seconds, milliseconds = divmod(remainder, 1000)
    return f"{hours:02}:{minutes:02}:{whole_seconds:02}.{milliseconds:03}"


plt.ion()
fig, ax = plt.subplots()
printed_runs = set()

while True:
    sizes = []
    times = []

    try:
        with open("benchmark.csv", newline="") as f:
            for row in csv.DictReader(f):
                size = int(row["size"])
                seconds = float(row["seconds"])
                sizes.append(size)
                times.append(seconds)

                run = (size, seconds)
                if run not in printed_runs:
                    print(
                        f"Cube size: {size}, duration: {seconds} seconds "
                        f"({format_duration(seconds)})"
                    )
                    printed_runs.add(run)
    except FileNotFoundError:
        pass

    ax.clear()

    if sizes:
        ax.plot(sizes, times, "o-")
        # ax.set_xscale("log", base=2)
        # ax.set_yscale("log")
        ax.set_xlabel("Cube size")
        ax.set_ylabel("Solve time (seconds)")
        ax.set_title("NxNxN Cube Solver Benchmark")
        ax.set_xticks(sizes)
        ax.grid(True)

    fig.canvas.draw_idle()
    fig.canvas.flush_events()
    time.sleep(1)
