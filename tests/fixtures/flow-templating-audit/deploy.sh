#!/bin/bash
# Producer HTTP via curl
curl -X POST https://api.example.com/deploy -d '{"env":"prod"}'
wget https://api.example.com/health
