# Source this to build Brindle on a machine without system dev packages.
# Provides a rootless gcc/pkg-config/dev-header toolchain extracted from .debs.
_TC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/root"
export PATH="$_TC/usr/bin:$PATH"
export CPATH="$_TC/usr/include:$_TC/usr/include/x86_64-linux-gnu${CPATH:+:$CPATH}"
export LIBRARY_PATH="$_TC/usr/lib/x86_64-linux-gnu${LIBRARY_PATH:+:$LIBRARY_PATH}"
export PKG_CONFIG_PATH="$_TC/usr/lib/x86_64-linux-gnu/pkgconfig:$_TC/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$_TC"
export LD_LIBRARY_PATH="$_TC/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export CC=gcc CXX=g++
export RUSTFLAGS="${RUSTFLAGS:-} -L native=$_TC/usr/lib/x86_64-linux-gnu"
unset _TC
