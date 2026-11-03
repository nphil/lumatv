//! Shared by the compositor (which draws `Content::Mesh`) and the apps (which place things over it): the camera
//! maths, so that a label an app projects onto a mesh layer lands exactly where the compositor draws that point,
//! and the procedural floor plan the graphics lab uses as its 3D test.

pub mod camera;
pub mod floorplan;
