import { connection } from '@kit.NetworkKit';

// A VPN network is never a valid egress for its own proxy sockets.
export async function getPhysicalNetwork(): Promise<connection.NetHandle> {
  const preferred = await connection.getDefaultNet();
  const preferredCaps = await connection.getNetCapabilities(preferred);
  if (preferred.netId > 0 && preferredCaps.networkCap?.includes(connection.NetCap.NET_CAPABILITY_NOT_VPN) && preferredCaps.networkCap?.includes(connection.NetCap.NET_CAPABILITY_INTERNET)) {
    return preferred;
  }
  const networks = await connection.getAllNets();
  for (const network of networks) {
    const caps = await connection.getNetCapabilities(network);
    if (network.netId > 0 && caps.networkCap?.includes(connection.NetCap.NET_CAPABILITY_NOT_VPN) &&
      caps.networkCap?.includes(connection.NetCap.NET_CAPABILITY_INTERNET)) {
      return network;
    }
  }
  throw new Error('no physical Internet network is available');
}
