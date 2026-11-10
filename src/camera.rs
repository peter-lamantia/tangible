use glam::{Mat4, Vec3};

/// Orbit camera that rotates around a target point.
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub aspect: f32,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
}

impl Camera {
    pub fn new(target: Vec3, distance: f32) -> Self {
        Self {
            target,
            distance,
            yaw: 0.6,
            pitch: 0.4,
            aspect: 16.0 / 9.0,
            fov: 45.0_f32.to_radians(),
            near: 0.001,
            far: 10000.0,
            orthographic: false,
        }
    }

    /// World-space eye position.
    pub fn eye(&self) -> Vec3 {
        let x = self.distance * self.pitch.cos() * self.yaw.sin();
        let y = self.distance * self.pitch.sin();
        let z = self.distance * self.pitch.cos() * self.yaw.cos();
        self.target + Vec3::new(x, y, z)
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    pub fn projection_matrix(&self) -> Mat4 {
        let near = (self.distance * 0.01).max(self.near);
        let far = (self.distance * 50.0).max(self.far);
        if self.orthographic {
            let half_height = self.distance * (self.fov * 0.5).tan();
            Mat4::orthographic_rh(
                -half_height * self.aspect,
                half_height * self.aspect,
                -half_height,
                half_height,
                near,
                far,
            )
        } else {
            Mat4::perspective_rh(self.fov, self.aspect, near, far)
        }
    }

    /// Calculate the ideal distance to frame an object of given bounding extent fully within the window.
    pub fn fit_distance(&self, extent: f32) -> f32 {
        let radius = extent * 0.5;
        let v_half_fov = self.fov * 0.5;
        let h_half_fov = (self.aspect * v_half_fov.tan()).atan();
        let min_half_fov = v_half_fov.min(h_half_fov);
        // Margin factor (1.35) ensures comfortable padding and accounts for top/bottom UI panels
        let margin = 1.35;
        (radius / min_half_fov.sin() * margin).max(0.01)
    }

    /// Reset the camera view to frame the target extent completely inside the window.
    pub fn frame_target(&mut self, target: Vec3, extent: f32) {
        self.target = target;
        self.distance = self.fit_distance(extent);
        self.yaw = 0.6;
        self.pitch = 0.4;
    }

    /// Rotate around the target (left-mouse drag).
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(-1.5, 1.5);
    }

    /// Zoom in/out (scroll wheel).
    pub fn zoom(&mut self, delta: f32) {
        self.distance *= 1.0 - delta * 0.1;
        self.distance = self.distance.max(0.001);
    }

    /// Pan the target point (right-mouse drag).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = Vec3::Y;
        let scale = self.distance * 0.002;
        self.target -= right * dx * scale;
        self.target += up * dy * scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orthographic_projection_keeps_parallel_lines_parallel() {
        let mut camera = Camera::new(Vec3::ZERO, 10.0);
        camera.orthographic = true;
        let projection = camera.projection_matrix();
        let near = projection * glam::Vec4::new(1.0, 0.0, -2.0, 1.0);
        let far = projection * glam::Vec4::new(1.0, 0.0, -8.0, 1.0);
        assert!((near.x / near.w - far.x / far.w).abs() < 1e-6);
    }
}
