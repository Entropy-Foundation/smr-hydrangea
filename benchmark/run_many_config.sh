#!/bin/bash

# Run config. SET BEFORE USE.
BLOCK_SIZES=() # E.g. (10 100 1000 10000)
DEBUG= # E.g. true
DURATION= # E.g. 300
GIT_BRANCHES=() # E.g. ("chained-moonshot" "commit-moonshot" "simple-moonshot")
NODES= # E.g. 10
RUNS= # E.g. 3
FAULTS= # Number of crashed nodes. E.g. 3
LEADER_ELECTOR= # FailureBestCase | FailureMidCase | FailureWorstCase | FairSuccession
