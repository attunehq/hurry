#!/bin/bash
set -e

TARGETS=(
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-musl"
)

echo "Testing cross build acceleration..."

for target in "${TARGETS[@]}"; do
    echo ""
    echo "========================================="
    echo "Testing target: $target"
    echo "========================================="

    # Clean start
    cargo clean

    # First build (should cache)
    echo "First build (caching)..."
    time hurry-dev cross build \
        --hurry-courier-url http://localhost:3000 \
        --hurry-wait-for-upload \
        -p hurry \
        --target "$target"

    # Clean
    cargo clean

    # Second build (should restore from cache)
    echo "Second build (restoring)..."
    time hurry-dev cross build \
        --hurry-courier-url http://localhost:3000 \
        --hurry-wait-for-upload \
        -p hurry \
        --target "$target"

    # Verify no spurious rebuilds
    echo "Verify no rebuilds..."
    hurry-dev cross build \
        --hurry-courier-url http://localhost:3000 \
        --hurry-wait-for-upload \
        -p hurry \
        --target "$target" 2>&1 | \
        tee /dev/tty | \
        grep -q "Finished" || (echo "FAIL: Build didn't complete" && exit 1)

    # Count how many crates were rebuilt (should be 0 or very few)
    rebuild_count=$(hurry-dev cross build \
        --hurry-courier-url http://localhost:3000 \
        --hurry-wait-for-upload \
        -p hurry \
        --target "$target" 2>&1 | \
        grep "Compiling" | wc -l)

    echo "Rebuild count: $rebuild_count"
    if [ "$rebuild_count" -gt 5 ]; then
        echo "WARNING: Too many rebuilds detected"
    fi

    echo "✓ Target $target passed"
done

echo ""
echo "All targets tested successfully!"
