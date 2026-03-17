set xdata time
set timefmt "%s"

plot \
    'm200.shard-00-00.ListenOverflows.rates' using 1:2, \
    'm200.shard-00-01.ListenOverflows.rates' using 1:2, \
    'm200.shard-00-02.ListenOverflows.rates' using 1:2, \
