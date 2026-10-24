//! STEP is the exchange format: geometry only, no feature history.
//! The editable history lives in `.smp` files (see `sm-format`).

use sm_kernel::Model;
use truck_stepio::out::{CompleteStepDisplay, StepHeaderDescriptor, StepModels};

/// Writes every body of `model` to a STEP Part 21 string.
pub fn export(model: &Model, file_name: &str) -> String {
    let compressed: Vec<_> = model.bodies.iter().map(|b| b.solid.compress()).collect();
    let models: StepModels<_, _, _> = compressed.iter().collect();
    let header = StepHeaderDescriptor {
        file_name: file_name.to_string(),
        organization_system: "SolidModeller".to_string(),
        ..Default::default()
    };
    CompleteStepDisplay::new(models, header).to_string()
}
