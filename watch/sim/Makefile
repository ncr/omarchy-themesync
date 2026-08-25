# Host-side simulated watch: the firmware's decoder compiled for the desktop.
CC ?= cc
CFLAGS ?= -std=c11 -O2 -Wall -Wextra -Werror -I../common

theme_sim: theme_sim.c ../common/theme_proto.c ../common/theme_proto.h
	$(CC) $(CFLAGS) -o $@ theme_sim.c ../common/theme_proto.c

test: theme_sim
	./theme_sim --selftest

clean:
	rm -f theme_sim
