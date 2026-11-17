#!/usr/bin/env bash
set -euo pipefail

dir="$(readlink -f "$1")"
export GST_REGISTRY_1_0="$(mktemp -d)/registry.bin"

pipeline() {
  gst-launch-1.0 -q videotestsrc num-buffers=5 \
    ! capsfilter caps="video/x-raw,width=320,height=240,format=DMA_DRM" \
    ! videoconvert ! fakesink
}

if pipeline >/dev/null 2>&1; then
  echo "a DMA_DRM capsfilter in system memory links without the fix, so this check proves nothing"
  exit 1
fi

if ! GST_PLUGIN_PATH="$dir" GST_TRACERS=commycamerafix pipeline; then
  echo "the camera fix did not repair a DMA_DRM capsfilter in system memory"
  exit 1
fi

if GST_PLUGIN_PATH="$dir" GST_TRACERS=commycamerafix gst-launch-1.0 videotestsrc num-buffers=1 ! fakesink 2>&1 | grep -q "no tracer named"; then
  echo "GStreamer did not find the commycamerafix tracer"
  exit 1
fi

echo "the camera fix repairs DMA_DRM capsfilters in system memory"
