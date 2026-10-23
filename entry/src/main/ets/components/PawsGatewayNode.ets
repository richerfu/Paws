import { ComponentContent, UIContext } from '@kit.ArkUI';

@Builder
function gatewayNode() {
  Column().width(0).height(0).accessibilityLevel('no');
}

export function createGatewayNode(context: UIContext): ComponentContent<Object> {
  return new ComponentContent<Object>(context, wrapBuilder(gatewayNode));
}
