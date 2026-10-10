"""Writes small Parquet fixtures for the payload decoder tests.
Regenerate: docker run --rm -v "$PWD":/w -w /w python:3.12-slim sh -c "pip -q install pyarrow==17.0.0 && python make_parquet.py"
"""
import datetime as dt
import decimal
import pyarrow as pa
import pyarrow.parquet as pq

n = 500
base = dt.datetime(2026, 10, 8, 10, 20, tzinfo=dt.timezone.utc)
table = pa.table({
    "Timestamp": pa.array([base + dt.timedelta(seconds=i) for i in range(n)], pa.timestamp("ms", tz="UTC")),
    "TurbineId": pa.array([f"T{(i % 8) + 1:02d}" for i in range(n)]),
    "ActivePower": pa.array([(i * 37.5) % 3600 - 12.5 for i in range(n)], pa.float64()),
    "WindSpeed": pa.array([None if i % 50 == 7 else 2.0 + (i % 130) / 10 for i in range(n)], pa.float32()),
    "Rpm": pa.array([9 + i % 7 for i in range(n)], pa.int32()),
    "Counter": pa.array([10_000_000_000 + i for i in range(n)], pa.int64()),
    "Running": pa.array([i % 9 != 0 for i in range(n)]),
    "Status": pa.array(["RUNNING" if i % 9 else "IDLE" for i in range(n)]),
    "Day": pa.array([dt.date(2026, 10, 8) for _ in range(n)], pa.date32()),
    "Price": pa.array([decimal.Decimal(i) / 100 for i in range(n)], pa.decimal128(9, 2)),
})
meta = {b"ProviderName": b"TurbineFastlog", b"Version": b"1.0.0"}
table = table.replace_schema_metadata(meta)

for codec in ["snappy", "gzip", "zstd", "lz4", "none"]:
    pq.write_table(table, f"turbine-{codec}.parquet", compression=codec, row_group_size=200)
pq.write_table(table, "turbine-plain-v2.parquet", compression="snappy", use_dictionary=False,
               data_page_version="2.0", row_group_size=200)
nested = pa.table({"id": pa.array([1, 2, 3]), "tags": pa.array([["a"], ["b", "c"], []])})
pq.write_table(nested, "nested.parquet", compression="snappy")
