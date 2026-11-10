# Source me (bash or zsh):  . yocto/env.sh
# Sets up the pinned buildtools and the bitbake environment, then cds into build/.
if [ -n "$BASH_SOURCE" ]; then _cse_y=$(cd "$(dirname "$BASH_SOURCE")" && pwd)
else _cse_y=${${(%):-%x}:A:h}; fi

. "$_cse_y/buildtools/environment-setup-x86_64-pokysdk-linux"
# host tools missing from both the distro and buildtools (diffstat) come from the distro package, see README.
TEMPLATECONF="$_cse_y/meta-cse/conf/templates/cse" \
  . "$_cse_y/layers/openembedded-core/oe-init-build-env" "$_cse_y/build"
unset _cse_y
