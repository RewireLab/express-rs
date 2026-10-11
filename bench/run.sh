#!/usr/bin/env sh
# Benchmark harness: express-rs vs Express 5, endpoint for endpoint.
#
# Methodology: autocannon with 8 connections, 2s warmup, 10s measured,
# both servers running back-to-back on the same machine. Absolute
# numbers are machine-specific; the comparison is the point.

set -e
cd "$(dirname "$0")/.."

PORT_RS=3001
PORT_JS=3002
CONNECTIONS=8
WARMUP=2
DURATION=10
ENDPOINTS="/hello /json /user/42 /chain"

echo "Building express-rs (release)..."
cargo build --release --example bench_server

echo "Starting servers..."
cargo run --release --example bench_server &
RS_PID=$!
node bench/express_app.js &
JS_PID=$!
trap 'kill $RS_PID $JS_PID 2>/dev/null' EXIT

wait_for() {
  for i in $(seq 1 50); do
    curl -s -o /dev/null "http://127.0.0.1:$1/hello" && return 0
    sleep 0.2
  done
  echo "server on $1 did not start" >&2
  exit 1
}
wait_for $PORT_RS
wait_for $PORT_JS

printf "%-8s %-8s %10s %8s %8s %8s\n" FRAMEWORK ENDPOINT "REQ/SEC" "P50(ms)" "P99(ms)" "ERRORS"
for ep in $ENDPOINTS; do
  # Warmup.
  npx --yes autocannon@8 -c $CONNECTIONS -d $WARMUP "http://127.0.0.1:$PORT_RS$ep" > /dev/null 2>&1
  npx --yes autocannon@8 -c $CONNECTIONS -d $WARMUP "http://127.0.0.1:$PORT_JS$ep" > /dev/null 2>&1

  for pair in "express-rs $PORT_RS" "express $PORT_JS"; do
    name=${pair%% *}
    port=${pair##* }
    npx --yes autocannon@8 -c $CONNECTIONS -d $DURATION --json "http://127.0.0.1:$port$ep" 2>/dev/null |
      node -e '
        let raw = "";
        process.stdin.on("data", (c) => (raw += c));
        process.stdin.on("end", () => {
          const r = JSON.parse(raw);
          const row = [
            process.argv[1],
            process.argv[2],
            r.requests.average.toFixed(0),
            r.latency.p50.toFixed(1),
            r.latency.p99.toFixed(1),
            r.errors + r.non2xx,
          ];
          console.log(row.join("\t"));
        });
      ' "$name" "$ep"
  done
done
