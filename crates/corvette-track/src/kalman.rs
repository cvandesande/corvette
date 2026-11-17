const POSITION_VARIANCE: f64 = 10.0; // norfair filter.py 250-264, factory default pos_variance
const POSITION_VELOCITY_COVARIANCE: f64 = 0.0; // norfair filter.py 250-264, factory default pos_vel_covariance
const VELOCITY_VARIANCE: f64 = 1.0; // norfair filter.py 250-264, factory default vel_variance

#[derive(Clone, Copy, Debug, PartialEq)]
struct Axis {
    position: f64,
    velocity: f64,
    position_variance: f64,
    position_velocity_covariance: f64,
    velocity_variance: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kalman {
    axes: [Axis; 4],
    r: f64,
    q: f64,
}

impl Kalman {
    #[must_use]
    pub fn new(initial: [f64; 4], r: f64, q: f64) -> Self {
        let axes = initial.map(|position| Axis {
            position,
            velocity: 0.0,
            position_variance: POSITION_VARIANCE,
            position_velocity_covariance: POSITION_VELOCITY_COVARIANCE,
            velocity_variance: VELOCITY_VARIANCE,
        });
        Self { axes, r, q }
    }

    pub fn predict(&mut self) {
        for axis in &mut self.axes {
            axis.position += axis.velocity;
        }
    }

    pub fn update(&mut self, measurement: [f64; 4]) {
        for (axis, measured) in self.axes.iter_mut().zip(measurement) {
            let error = measured - axis.position;
            let pos_plus_cov = axis.position_variance + axis.position_velocity_covariance;
            let vel_plus_cov = axis.position_velocity_covariance + axis.velocity_variance;
            let sum_a = pos_plus_cov + vel_plus_cov;
            let sum_b = sum_a + self.q;
            let added = sum_b + self.r;
            let r_over = self.r / added;
            let cov_over = vel_plus_cov / added;
            let not_r_over = 1.0 - r_over;
            let position_correction = not_r_over * error;
            let velocity_correction = cov_over * error;
            let new_position = axis.position + position_correction;
            let new_velocity = axis.velocity + velocity_correction;
            let new_position_variance = not_r_over * self.r;
            let new_position_velocity_covariance = cov_over * self.r;
            let cov_squared_over = (cov_over * cov_over) * added;
            let variance_delta = self.q - cov_squared_over;
            let new_velocity_variance = axis.velocity_variance + variance_delta;
            axis.position = new_position;
            axis.velocity = new_velocity;
            axis.position_variance = new_position_variance;
            axis.position_velocity_covariance = new_position_velocity_covariance;
            axis.velocity_variance = new_velocity_variance;
        }
    }

    #[must_use]
    pub fn position(&self) -> [f64; 4] {
        self.axes.map(|axis| axis.position)
    }

    #[must_use]
    pub fn velocity(&self) -> [f64; 4] {
        self.axes.map(|axis| axis.velocity)
    }

    #[must_use]
    pub fn position_variance(&self) -> [f64; 4] {
        self.axes.map(|axis| axis.position_variance)
    }

    #[must_use]
    pub fn position_velocity_covariance(&self) -> [f64; 4] {
        self.axes.map(|axis| axis.position_velocity_covariance)
    }

    #[must_use]
    pub fn velocity_variance(&self) -> [f64; 4] {
        self.axes.map(|axis| axis.velocity_variance)
    }
}
