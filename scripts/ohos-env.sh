#!/usr/bin/env sh
# Source this file to select one matching DevEco SDK, JBR and build tool set.
if [ -z "${DEVECO_STUDIO_HOME:-}" ]; then
  if [ -n "${OHOS_NDK_HOME:-}" ] && [ -d "$OHOS_NDK_HOME/../../../tools/hvigor" ]; then
    DEVECO_STUDIO_HOME=$(CDPATH= cd -- "$OHOS_NDK_HOME/../../.." && pwd)
  else
    for paws_studio in \
      /Applications/DevEco-Studio.app/Contents \
      "$HOME/.meat/ide/DevEco-Studio-26.0.0.821.app/Contents" \
      "$HOME/.meat/ide/DevEco-Studio-6.1.1.280.app/Contents"
    do
      if [ -d "$paws_studio/tools/hvigor" ]; then
        DEVECO_STUDIO_HOME=$paws_studio
        break
      fi
    done
  fi
fi
if [ -n "${DEVECO_STUDIO_HOME:-}" ]; then
  export DEVECO_STUDIO_HOME
  export DEVECO_SDK_HOME="$DEVECO_STUDIO_HOME/sdk"
  export OHOS_NDK_HOME="${OHOS_NDK_HOME:-$DEVECO_SDK_HOME/default/openharmony}"
  if [ -d "$DEVECO_STUDIO_HOME/jbr/Contents/Home" ]; then
    export JAVA_HOME="${DEVECO_JAVA_HOME:-$DEVECO_STUDIO_HOME/jbr/Contents/Home}"
    export PATH="$JAVA_HOME/bin:$PATH"
  fi
  export PATH="$DEVECO_STUDIO_HOME/tools/node/bin:$DEVECO_STUDIO_HOME/tools/ohpm/bin:$DEVECO_STUDIO_HOME/tools/hvigor/bin:$PATH"
fi
