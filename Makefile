.PHONY: all
all: m200.shard-00-00.CPU%.tab m200.shard-00-01.CPU%.tab m200.shard-00-02.CPU%.tab

m200.shard-00-00.CPU\%.tab m200.shard-00-01.CPU\%.tab m200.shard-00-02.CPU\%.tab: /Users/david.goffredo/Documents/connection-rate-limiter/dsi-runs/run6/m200/ftdc
	./cpu-percents $<

