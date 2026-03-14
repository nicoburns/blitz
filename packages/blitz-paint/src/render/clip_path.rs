use super::ElementCx;
use kurbo::{BezPath, Point, Shape};
use style::values::computed::basic_shape::{BasicShape, ClipPath};
use style::values::computed::{CSSPixelLength, LengthPercentage};
use style::values::generics::basic_shape::{
    AxisEndPoint, AxisPosition, CommandEndPoint, ControlPoint, GenericBasicShape,
    GenericPathOrShapeFunction, GenericShapeRadius, Path, ShapeBox, ShapeGeometryBox,
};
use style::values::generics::position::GenericPositionOrAuto;

impl ElementCx<'_> {
    /// Compute the clip-path BezPath (if any) for this element.
    /// Returns `None` if clip-path is `none` or unsupported.
    pub(super) fn clip_path_shape(&self) -> Option<BezPath> {
        let clip_path = self.style.clone_clip_path();
        match clip_path {
            ClipPath::None => None,
            ClipPath::Url(_) => {
                // URL references (e.g. `clip-path: url(#myClip)`) are not yet supported
                None
            }
            ClipPath::Shape(basic_shape, geometry_box) => {
                let reference_box = self.resolve_geometry_box(&geometry_box);
                self.basic_shape_to_path(&basic_shape, reference_box)
            }
            ClipPath::Box(geometry_box) => {
                let reference_box = self.resolve_geometry_box(&geometry_box);
                Some(rect_to_path(
                    reference_box.x,
                    reference_box.y,
                    reference_box.width,
                    reference_box.height,
                ))
            }
        }
    }

    /// Resolve a ShapeGeometryBox to a concrete rectangle (x, y, width, height) in scaled pixels
    fn resolve_geometry_box(&self, geometry_box: &ShapeGeometryBox) -> ReferenceBox {
        match geometry_box {
            ShapeGeometryBox::ElementDependent
            | ShapeGeometryBox::ShapeBox(ShapeBox::BorderBox) => ReferenceBox {
                x: 0.0,
                y: 0.0,
                width: self.frame.border_box.width(),
                height: self.frame.border_box.height(),
            },
            ShapeGeometryBox::ShapeBox(ShapeBox::PaddingBox) => ReferenceBox {
                x: self.frame.border_width.x0,
                y: self.frame.border_width.y0,
                width: self.frame.padding_box.width(),
                height: self.frame.padding_box.height(),
            },
            ShapeGeometryBox::ShapeBox(ShapeBox::ContentBox) => ReferenceBox {
                x: self.frame.border_width.x0 + self.frame.padding_width.x0,
                y: self.frame.border_width.y0 + self.frame.padding_width.y0,
                width: self.frame.content_box.width(),
                height: self.frame.content_box.height(),
            },
            ShapeGeometryBox::ShapeBox(ShapeBox::MarginBox) => {
                // Margin box is not tracked in CssBox, fall back to border box
                ReferenceBox {
                    x: 0.0,
                    y: 0.0,
                    width: self.frame.border_box.width(),
                    height: self.frame.border_box.height(),
                }
            }
            // SVG geometry boxes - fall back to border box for HTML elements
            ShapeGeometryBox::FillBox | ShapeGeometryBox::StrokeBox | ShapeGeometryBox::ViewBox => {
                ReferenceBox {
                    x: 0.0,
                    y: 0.0,
                    width: self.frame.border_box.width(),
                    height: self.frame.border_box.height(),
                }
            }
        }
    }

    /// Convert a CSS basic-shape to a kurbo BezPath
    fn basic_shape_to_path(
        &self,
        shape: &BasicShape,
        reference_box: ReferenceBox,
    ) -> Option<BezPath> {
        let w = reference_box.width;
        let h = reference_box.height;
        let ox = reference_box.x;
        let oy = reference_box.y;

        match shape {
            GenericBasicShape::Circle(circle) => {
                let (cx, cy) = resolve_position(&circle.position, w, h, ox, oy);
                let r = resolve_shape_radius(&circle.radius, w, h, cx - ox, cy - oy);
                Some(circle_path(cx, cy, r))
            }
            GenericBasicShape::Ellipse(ellipse) => {
                let (cx, cy) = resolve_position(&ellipse.position, w, h, ox, oy);
                let rx = resolve_shape_radius(&ellipse.semiaxis_x, w, h, cx - ox, cy - oy);
                let ry = resolve_shape_radius(&ellipse.semiaxis_y, h, w, cy - oy, cx - ox);
                Some(ellipse_path(cx, cy, rx, ry))
            }
            GenericBasicShape::Polygon(polygon) => {
                let mut path = BezPath::new();
                let _fill = &polygon.fill;
                let coords = &polygon.coordinates;

                if coords.is_empty() {
                    return None;
                }

                for (i, coord) in coords.iter().enumerate() {
                    let x = ox + resolve_lp(&coord.0, w);
                    let y = oy + resolve_lp(&coord.1, h);
                    if i == 0 {
                        path.move_to(Point::new(x, y));
                    } else {
                        path.line_to(Point::new(x, y));
                    }
                }
                path.close_path();

                Some(path)
            }
            GenericBasicShape::Rect(inset_rect) => {
                // inset() function: inset(top right bottom left round border-radius)
                let top = resolve_lp(&inset_rect.rect.0, h);
                let right = resolve_lp(&inset_rect.rect.1, w);
                let bottom = resolve_lp(&inset_rect.rect.2, h);
                let left = resolve_lp(&inset_rect.rect.3, w);

                let x0 = ox + left;
                let y0 = oy + top;
                let x1 = ox + w - right;
                let y1 = oy + h - bottom;

                if x1 <= x0 || y1 <= y0 {
                    return None;
                }

                // TODO: Support border-radius on inset()
                Some(rect_to_path(x0, y0, x1 - x0, y1 - y0))
            }
            GenericBasicShape::PathOrShape(path_or_shape) => match path_or_shape {
                GenericPathOrShapeFunction::Path(path) => svg_path_to_bezpath(path).map(|mut p| {
                    p.apply_affine(kurbo::Affine::translate((ox, oy)));
                    p
                }),
                GenericPathOrShapeFunction::Shape(_shape) => {
                    // shape() function is complex; not yet supported
                    None
                }
            },
        }
    }
}

/// A resolved reference box in scaled pixel coordinates
#[derive(Clone, Copy)]
struct ReferenceBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Resolve a LengthPercentage value against a basis length (already scaled)
fn resolve_lp(lp: &LengthPercentage, basis: f64) -> f64 {
    let basis_css = CSSPixelLength::new(basis as f32);
    lp.resolve(basis_css).px() as f64
}

/// Resolve a position (or auto => center) to absolute coordinates
fn resolve_position(
    position: &GenericPositionOrAuto<style::values::computed::Position>,
    w: f64,
    h: f64,
    ox: f64,
    oy: f64,
) -> (f64, f64) {
    match position {
        GenericPositionOrAuto::Auto => (ox + w / 2.0, oy + h / 2.0),
        GenericPositionOrAuto::Position(pos) => {
            let x = ox + resolve_lp(&pos.horizontal, w);
            let y = oy + resolve_lp(&pos.vertical, h);
            (x, y)
        }
    }
}

/// Resolve a shape radius keyword or length value
fn resolve_shape_radius(
    radius: &GenericShapeRadius<LengthPercentage>,
    primary_size: f64,
    secondary_size: f64,
    center_offset_primary: f64,
    center_offset_secondary: f64,
) -> f64 {
    match radius {
        GenericShapeRadius::Length(lp) => resolve_lp(&lp.0, primary_size),
        GenericShapeRadius::ClosestSide => center_offset_primary
            .min(primary_size - center_offset_primary)
            .min(center_offset_secondary)
            .min(secondary_size - center_offset_secondary)
            .max(0.0),
        GenericShapeRadius::FarthestSide => center_offset_primary
            .max(primary_size - center_offset_primary)
            .max(center_offset_secondary)
            .max(secondary_size - center_offset_secondary),
    }
}

/// Build a rectangle BezPath
fn rect_to_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let rect = kurbo::Rect::new(x, y, x + w, y + h);
    let mut path = BezPath::new();
    path.move_to(Point::new(rect.x0, rect.y0));
    path.line_to(Point::new(rect.x1, rect.y0));
    path.line_to(Point::new(rect.x1, rect.y1));
    path.line_to(Point::new(rect.x0, rect.y1));
    path.close_path();
    path
}

/// Build a circle BezPath using cubic Bézier approximation
fn circle_path(cx: f64, cy: f64, r: f64) -> BezPath {
    ellipse_path(cx, cy, r, r)
}

/// Build an ellipse BezPath using cubic Bézier approximation
fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> BezPath {
    let ellipse = kurbo::Ellipse::new(Point::new(cx, cy), (rx, ry), 0.0);
    BezPath::from_vec(ellipse.path_elements(0.1).collect())
}

/// Convert an SVG path() to a kurbo BezPath.
/// The returned path is in the path's own coordinate system (origin at 0,0).
/// The caller is responsible for translating it to the reference box origin.
fn svg_path_to_bezpath(path: &Path) -> Option<BezPath> {
    use style::values::specified::svg_path::PathCommand;

    let commands = path.commands();
    if commands.is_empty() {
        return None;
    }

    let mut bez = BezPath::new();
    let mut cur = Point::ZERO;
    let mut subpath_start = cur;

    for cmd in commands {
        match cmd {
            PathCommand::Close => {
                bez.close_path();
                cur = subpath_start;
            }
            PathCommand::Move { point } => {
                let p = resolve_endpoint(point, cur);
                bez.move_to(p);
                cur = p;
                subpath_start = p;
            }
            PathCommand::Line { point } => {
                let p = resolve_endpoint(point, cur);
                bez.line_to(p);
                cur = p;
            }
            PathCommand::HLine { x } => {
                cur.x = resolve_axis_endpoint(x, cur.x);
                bez.line_to(cur);
            }
            PathCommand::VLine { y } => {
                cur.y = resolve_axis_endpoint(y, cur.y);
                bez.line_to(cur);
            }
            PathCommand::CubicCurve {
                point,
                control1,
                control2,
            } => {
                let p = resolve_endpoint(point, cur);
                let c1 = resolve_control_point(control1, cur);
                let c2 = resolve_control_point(control2, cur);
                bez.curve_to(c1, c2, p);
                cur = p;
            }
            PathCommand::QuadCurve { point, control1 } => {
                let p = resolve_endpoint(point, cur);
                let c1 = resolve_control_point(control1, cur);
                bez.quad_to(c1, p);
                cur = p;
            }
            PathCommand::SmoothCubic { point, control2 } => {
                // For smooth cubic, control1 is reflection of previous control2
                // Simplified: use current point as control1
                let p = resolve_endpoint(point, cur);
                let c2 = resolve_control_point(control2, cur);
                bez.curve_to(cur, c2, p);
                cur = p;
            }
            PathCommand::SmoothQuad { point } => {
                // Simplified: treat as line
                let p = resolve_endpoint(point, cur);
                bez.line_to(p);
                cur = p;
            }
            PathCommand::Arc { point, .. } => {
                // SVG arc commands are complex; approximate as a line for now
                let p = resolve_endpoint(point, cur);
                bez.line_to(p);
                cur = p;
            }
        }
    }

    Some(bez)
}

/// Resolve a CommandEndPoint to an absolute Point.
/// `ToPosition` is absolute; `ByCoordinate` is relative to `cur`.
fn resolve_endpoint(
    ep: &CommandEndPoint<style::values::generics::position::GenericPosition<f32, f32>, f32>,
    cur: Point,
) -> Point {
    match ep {
        CommandEndPoint::ToPosition(pos) => Point::new(pos.horizontal as f64, pos.vertical as f64),
        CommandEndPoint::ByCoordinate(coord) => {
            Point::new(cur.x + coord.x as f64, cur.y + coord.y as f64)
        }
    }
}

/// Resolve a ControlPoint to an absolute Point.
/// `Absolute` is absolute; `Relative` is relative to `cur`.
fn resolve_control_point(
    cp: &ControlPoint<style::values::generics::position::GenericPosition<f32, f32>, f32>,
    cur: Point,
) -> Point {
    match cp {
        ControlPoint::Absolute(pos) => Point::new(pos.horizontal as f64, pos.vertical as f64),
        ControlPoint::Relative(rel) => {
            Point::new(cur.x + rel.coord.x as f64, cur.y + rel.coord.y as f64)
        }
    }
}

/// Resolve an AxisEndPoint to an absolute value.
/// `ToPosition` is absolute; `ByCoordinate` is relative to `cur_val`.
fn resolve_axis_endpoint(ep: &AxisEndPoint<f32>, cur_val: f64) -> f64 {
    match ep {
        AxisEndPoint::ToPosition(AxisPosition::LengthPercent(lp)) => *lp as f64,
        AxisEndPoint::ToPosition(AxisPosition::Keyword(_)) => cur_val,
        AxisEndPoint::ByCoordinate(val) => cur_val + *val as f64,
    }
}
