
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading.Tasks;

namespace Interoptopus
{
    public delegate void Run();

    public class MeasureResult
    {
        private static long _calibrationTicks = 0;

        long _n;
        long _totalTicks;

        public double MicroPer1000()
        {
            var n = (double) _n;
            var ticks_for_all_n = (double) _totalTicks;

            // ns/call == µs/1000 calls numerically; use Stopwatch.Frequency to be platform-independent
            var ns_per_call = ticks_for_all_n / n * 1_000_000_000.0 / Stopwatch.Frequency;
            return ns_per_call;
        }

        public MeasureResult(long n, long totalTicks)
        {
            _n = n;
            _totalTicks = totalTicks;
        }

        public static void Calibrate(uint n, Run r)
        {
            _calibrationTicks = 0;
            var result = Measure(n, r);
            _calibrationTicks = result._totalTicks;
        }

        public static MeasureResult Measure(uint n, Run r)
        {
            ArgumentOutOfRangeException.ThrowIfZero(n);
            for (var i = 0; i < n; i++)
            {
                r.Invoke();
            }

            var stopwatch = new Stopwatch();
            stopwatch.Start();
            for (var i = 0; i < n; i++)
            {
                r.Invoke();
            }
            stopwatch.Stop();

            return new MeasureResult(n, Math.Max(0, stopwatch.ElapsedTicks - _calibrationTicks));
        }

        public static async Task<MeasureResult> MeasureAsync(uint n, Func<Task> run)
        {
            ArgumentOutOfRangeException.ThrowIfZero(n);
            for (var i = 0u; i < n; ++i) await run();
            var stopwatch = Stopwatch.StartNew();
            for (var i = 0u; i < n; ++i) await run();
            stopwatch.Stop();
            // A synchronous empty-loop calibration does not model awaited continuations.
            return new MeasureResult(n, stopwatch.ElapsedTicks);
        }

    }

    class Entry
    {
        public string Name;
        public MeasureResult Result;
    }

    public class MarkdownTableWriter
    {
        private List<Entry> Entries = new List<Entry>();

        public void Add(string name, MeasureResult result)
        {
            Console.WriteLine($"{name}: {result.MicroPer1000():F0}");
            Entries.Add(new Entry()
            {
                Name = name,
                Result = result
            });
        }

        public void Write(string file, uint iterations)
        {
            var header = $@"
# FFI Call Overheads

Each entry runs {iterations} warmup calls followed by {iterations} measured calls.
Times include the native function's work. Async entries await every invocation to completion.
Synchronous entries subtract an empty-loop calibration; async entries report total elapsed time.
These ad-hoc timings are estimates, not statistically rigorous comparisons.

## System

- OS: {RuntimeInformation.OSDescription}
- Architecture: {RuntimeInformation.ProcessArchitecture}
- Runtime: {RuntimeInformation.FrameworkDescription}
- Stopwatch frequency: {Stopwatch.Frequency} Hz

## Results

| Construct | ns per call |
| --- | --- |
";

            using StreamWriter sw = File.CreateText(file);
            sw.Write(header);
            foreach (var entry in Entries)
            {
                sw.WriteLine($"| `{entry.Name}` | {(long) entry.Result.MicroPer1000():F0} |");
            }
        }
    }
}
