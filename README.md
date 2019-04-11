# closefds

closefds is a library that provides support for setting the `FD_CLOEXEC` flag on all open
file descriptors after `fork()` and before `exec()` on UNIX-like systems.

Any file descriptors that aren't marked with this flag will stay open after `exec()`
which can cause resources to leak and can lead to deadlocks. Ideally, whenever a file
descriptor is created, it will be created with the `FD_CLOEXEC` flag already set.
However, this may not be possible in some circumstances - such as when using an
external library or a system call that does not support the `FD_CLOEXEC` flag, such as
`pipe()`.

The function `close_fds_on_exec()` will create a closure that can be passed
as a `pre_exec()` function when spawning a child process via the `Command` interface
and will set the `FD_CLOEXEC` flag as appropriate on open file descriptors.
