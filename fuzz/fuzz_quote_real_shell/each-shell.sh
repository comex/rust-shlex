#!/usr/bin/env zsh
# Run a command for each of several configurations.
# Example:
# ./each-shell.sh 'cargo fuzz run --fuzz-dir . fuzz_quote_real_shell basic-corpus/*'
# ./each-shell.sh 'nohup cargo fuzz run --fuzz-dir . fuzz_quote_real_shell >&/tmp/out.$ident &'

# TODO: This could be handled better.  The choice of shell should probably just
# be part of the fuzz input.

shells=(
    'zsh --no-rcs'
    'bash --norc'
    'dash +m'
    'fish --private --no-config'
    'mksh'
)

running_on_linux=1
if [[ `uname` == Darwin && "$FUZZ_USE_DOCKER" == 0 ]]; then
    running_on_linux=0
fi
# Add busybox unless we're running natively on macOS, since busybox doesn't run
# on macOS.
# (If you're on Linux but it's not installed, then too bad, install it.)
if (( running_on_linux )); then
    shells+=('busybox ash +m')
fi
# Gather existing FUZZ_* environment variables just to make it easier to copy
# and paste individual commands from the debug output:
already_set=$(export | grep '^FUZZ_' | tr '\n' ' ')
for shell in $shells; do
    for interactive in '-i' '+i'; do
        for pty in 0 1; do
            for lang in C en_US.UTF-8; do
                ident="${shell%% *}.$interactive.pty$pty.$lang"
                if [[ $shell == fish* && $ident != fish.-i.pty1.* ]]; then
                    # fish must have a pty because otherwise it buffers the
                    # entire stdin rather than responding live; and it must
                    # have -i because +i doesn't actually work.
                    continue
                fi
                if [[ $ident == zsh.-i.pty0.* ]]; then
                    # zsh in interactive mode forces the use of the tty instead of
                    # using stdin/stdout, so we can't test it without a pty.
                    continue
                fi
                if [[ $ident == zsh.+i.pty1.* && $running_on_linux == 0 ]]; then
                    # Fails due to a macOS kernel bug(?).  In zsh, `shingetchar`
                    # really does not want to read past a newline.  Instead of just
                    # just buffering any excess data, it uses a weird scheme where
                    # it tries a no-op lseek on the input fd.  If that succeeds, it
                    # calls `read` with some reasonable buffer size and then, if it
                    # read too many bytes (i.e. past a newline), it lseeks
                    # backwards to the newline.  If the no-op lseek fails, it falls
                    # back to reading one byte at a time.  On macOS, lseek on a pty
                    # succeeds even though it does not do anything meaningful.
                    # Pipes don't have this issue.
                    continue
                fi
                prefix="FUZZ_USE_PTY=$pty FUZZ_SHELL=\"env LANG=$lang $shell $interactive\" "
                echo ">> ${already_set}ident='$ident' $prefix $*"
                eval "$prefix $*" || {
                    echo "FAIL: $ident"
                    exit 1
                }
            done
        done
    done
done
echo 'all ok'
