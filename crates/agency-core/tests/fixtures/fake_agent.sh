#!/usr/bin/env bash
# A stand-in terminal agent for tests: announce readiness, echo the prompt arg,
# read one line of input and echo it back, then exit cleanly.
echo "AGENT_READY"
echo "PROMPT:$1"
read -r line
echo "GOT:$line"
echo "AGENT_DONE"
