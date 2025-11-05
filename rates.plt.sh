#!/bin/sh

# m80.shard-00-01.rates


cat <<'END_GNUPLOT'
set xdata time
set timefmt "%s"

plot \
END_GNUPLOT

for f in *.rates; do
    cat <<END_GNUPLOT
    '$f' using 1:2, \\
END_GNUPLOT
done

