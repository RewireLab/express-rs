#!/usr/bin/env sh
# Benchmark harness: express-rs vs Express 5, endpoint for endpoint.
#
# Methodology: autocannon with 8 connections, 2s warmup, 10s measured,
# both servers running back-to-back on the same machine. Each endpoint
# must meet MIN_RATIO throughput and return zero errors/non-2xx responses.

set -e
cd "$(dirname "$0")/.."

PORT_RS=${PORT_RS:-3001}
PORT_JS=${PORT_JS:-3002}
CONNECTIONS=${CONNECTIONS:-8}
WARMUP=${WARMUP:-2}
DURATION=${DURATION:-10}
ENDPOINTS=${ENDPOINTS:-"/hello /json /user/42 /chain"}
MIN_RATIO=${MIN_RATIO:-1.5}

echo "Building express-rs (release)..."
cargo build --release --example bench_server

echo "Starting servers..."
PORT_RS="$PORT_RS" target/release/examples/bench_server &
RS_PID=$!
PORT_JS="$PORT_JS" node bench/express_app.js &
JS_PID=$!
cleanup() {
  kill "$RS_PID" "$JS_PID" 2>/dev/null || true
  wait "$RS_PID" "$JS_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

wait_for() {
  pid=$1
  port=$2
  name=$3
  for i in $(seq 1 50); do
    state=$(ps -o stat= -p "$pid" 2>/dev/null || true)
    case "$state" in
      ''|Z*)
        wait "$pid" 2>/dev/null || true
        echo "$name exited before becoming ready on port $port; check whether the port is already in use" >&2
        return 1
        ;;
    esac
    curl --max-time 1 -fs -o /dev/null "http://127.0.0.1:$port/hello" && return 0
    sleep 0.2
  done
  echo "$name did not become ready on port $port" >&2
  return 1
}
wait_for "$RS_PID" "$PORT_RS" "express-rs"
wait_for "$JS_PID" "$PORT_JS" "Express"

printf "%-8s %-8s %10s %8s %8s %8s\n" FRAMEWORK ENDPOINT "REQ/SEC" "P50(ms)" "P99(ms)" "ERRORS"
gate_failed=0
for ep in $ENDPOINTS; do
  # Warmup.
  npx --yes autocannon@8 -c $CONNECTIONS -d $WARMUP "http://127.0.0.1:$PORT_RS$ep" > /dev/null 2>&1
  npx --yes autocannon@8 -c $CONNECTIONS -d $WARMUP "http://127.0.0.1:$PORT_JS$ep" > /dev/null 2>&1

  rs_metrics=
  js_metrics=
  for pair in "express-rs $PORT_RS" "express $PORT_JS"; do
    name=${pair%% *}
    port=${pair##* }
    metrics=$(npx --yes autocannon@8 -c "$CONNECTIONS" -d "$DURATION" --json "http://127.0.0.1:$port$ep" 2>/dev/null |
      node -e '
        let raw = "";
        process.stdin.on("data", (c) => (raw += c));
        process.stdin.on("end", () => {
          const r = JSON.parse(raw);
          const row = [
            r.requests.average.toFixed(0),
            r.latency.p50.toFixed(1),
            r.latency.p99.toFixed(1),
            r.errors + r.non2xx,
          ];
          console.log(row.join(" "));
        });
      ')
    set -- $metrics
    req_sec=$1
    p50=$2
    p99=$3
    errors=$4
    printf "%-8s %-8s %10s %8s %8s %8s\n" "$name" "$ep" "$req_sec" "$p50" "$p99" "$errors"
    if [ "$name" = "express-rs" ]; then
      rs_metrics="$req_sec $errors"
    else
      js_metrics="$req_sec $errors"
    fi
  done

  set -- $rs_metrics
  rs_req_sec=$1
  rs_errors=$2
  set -- $js_metrics
  js_req_sec=$1
  js_errors=$2
  ratio=$(awk -v rs="$rs_req_sec" -v js="$js_req_sec" 'BEGIN { printf "%.2f", rs / js }')
  printf "%-8s %-8s %10s %8s\n" "ratio" "$ep" "${ratio}x" "min ${MIN_RATIO}x"
  if [ "$rs_errors" -ne 0 ] || [ "$js_errors" -ne 0 ] || [ "$js_req_sec" -le 0 ] || \
    awk -v rs="$rs_req_sec" -v js="$js_req_sec" -v min="$MIN_RATIO" 'BEGIN { exit !(rs < js * min) }'; then
    echo "performance gate failed for $ep" >&2
    gate_failed=1
  fi
done

if [ "$gate_failed" -ne 0 ]; then
  exit 1
fi
echo "Performance gate passed (minimum ratio ${MIN_RATIO}x, zero errors)."
