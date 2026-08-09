//! Initial safe-area snapshot captured by the ArkTS `paws.safe-area` plugin.

use arkit::openharmony_ability::{AvoidArea, AvoidAreaType, OpenHarmonyApp};
use arkit::EdgeInsets;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InitialSafeArea(pub(crate) EdgeInsets);

pub(crate) fn initial_safe_area(app: &OpenHarmonyApp) -> InitialSafeArea {
    let scale = normalized_scale(app.scale());
    InitialSafeArea(combine_visual_avoid_areas(
        [
            AvoidAreaType::System,
            AvoidAreaType::Cutout,
            AvoidAreaType::NavigationIndicator,
        ]
        .into_iter()
        .filter_map(|area_type| app.avoid_area(area_type)),
        scale,
    ))
}

fn normalized_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn combine_visual_avoid_areas(
    areas: impl IntoIterator<Item = AvoidArea>,
    scale: f32,
) -> EdgeInsets {
    areas
        .into_iter()
        .filter(|area| area.visible)
        .fold(EdgeInsets::ZERO, |insets, area| {
            insets.max(EdgeInsets {
                top: area.top_rect.height.max(0) as f32 / scale,
                right: area.right_rect.width.max(0) as f32 / scale,
                bottom: area.bottom_rect.height.max(0) as f32 / scale,
                left: area.left_rect.width.max(0) as f32 / scale,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkit::openharmony_ability::Rect;

    fn avoid(top: i32, right: i32, bottom: i32, left: i32, visible: bool) -> AvoidArea {
        AvoidArea {
            visible,
            top_rect: Rect {
                height: top,
                ..Rect::default()
            },
            right_rect: Rect {
                width: right,
                ..Rect::default()
            },
            bottom_rect: Rect {
                height: bottom,
                ..Rect::default()
            },
            left_rect: Rect {
                width: left,
                ..Rect::default()
            },
        }
    }

    #[test]
    fn initial_safe_area_combines_visible_edges_and_converts_px_to_vp() {
        let insets = combine_visual_avoid_areas(
            [
                avoid(90, 0, 0, 0, true),
                avoid(0, 30, 72, 24, true),
                avoid(300, 300, 300, 300, false),
            ],
            3.0,
        );

        assert_eq!(
            insets,
            EdgeInsets {
                top: 30.0,
                right: 10.0,
                bottom: 24.0,
                left: 8.0,
            }
        );
    }

    #[test]
    fn invalid_display_scale_falls_back_to_one() {
        assert_eq!(normalized_scale(0.0), 1.0);
        assert_eq!(normalized_scale(f32::NAN), 1.0);
    }
}
