/** Product geometry in logical points. Device identity never follows width. */
export function workspaceLayout(
  width: number,
  desktop: boolean,
  expandedPreference?: boolean,
  scale = 1,
  pageMax = Infinity,
) {
  const sidebar = desktop || width >= 840;
  // Even an explicitly expanded sidebar must leave a useful single pane.
  const expanded = sidebar && width >= 800 && (expandedPreference ?? width >= 1100);
  const sidebarWidth = sidebar ? (expanded ? 232 : 64) : 0;
  const inset = sidebar ? 20 : 16;
  const content = Math.max(0, width - sidebarWidth);
  const usable = Math.max(0, Math.min(content, pageMax) - inset * 2);
  const gap = sidebar ? (usable >= 640 ? 16 : 8) : 12;
  const textScale = Math.max(1, scale);
  const collectionMin = 240 * textScale;
  const detailMin = 320 * textScale;
  const masterDetail = (desktop || width >= 840) && usable >= collectionMin + gap + detailMin;
  const collection = Math.max(collectionMin, Math.min(340 * textScale, usable * 0.36, usable - gap - detailMin));
  const overview = (desktop || width >= 840) && usable >= 300 * textScale + gap + detailMin;
  const connection = Math.max(
    300 * textScale,
    Math.min(360 * textScale, Math.floor(usable * 0.39), usable - gap - detailMin),
  );
  return { sidebar, expanded, sidebarWidth, inset, gap, content, usable,
    masterDetail, collection, collectionMin, detailMin, overview,
    connection, tableDetails: usable >= 800 * textScale };
}
