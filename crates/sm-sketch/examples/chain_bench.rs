use sm_sketch::*;
use std::time::Instant;
fn main() {
    for n in [20usize, 100, 200, 400, 1000] {
        let mut s = Sketch::new();
        let mut prev = s.origin();
        let t = Instant::now();
        let mut dims = Vec::new();
        for i in 0..n {
            let [x, y] = s.point(prev).unwrap();
            let next = if i % 2 == 0 {
                s.add_point(x + 9.0, y + 0.5).unwrap()
            } else {
                s.add_point(x + 0.5, y + 9.0).unwrap()
            };
            let line = s.add_line(prev, next).unwrap();
            s.add_constraint(if i % 2 == 0 {
                Constraint::Horizontal(line)
            } else {
                Constraint::Vertical(line)
            })
            .unwrap();
            dims.push(
                s.add_constraint(Constraint::Distance(prev, next, 10.0))
                    .unwrap(),
            );
            prev = next;
        }
        let build = t.elapsed();
        let t = Instant::now();
        s.set_dimension(dims[0], 12.0).unwrap();
        let set = t.elapsed();
        // drag in an under-constrained version: drop the last dim
        s.remove_constraint(*dims.last().unwrap()).unwrap();
        let p = s.point(prev).unwrap();
        let t = Instant::now();
        s.drag_point(prev, [p[0] + 3.0, p[1] + 1.0]).unwrap();
        let drag = t.elapsed();
        let t = Instant::now();
        let a = s.analyse();
        let an = t.elapsed();
        println!("{n} lines: build {build:?}, set_dimension {set:?}, drag {drag:?}, analyse {an:?} (dof {})", a.dof);
    }
}
