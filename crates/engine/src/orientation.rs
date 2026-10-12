//! `Orientation`: the frame experimental redstone carries with each update.
//!
//! Under the Redstone Experiments feature, an update knows which way it is
//! travelling (`front`), which way is up, and which side it favours. Dust
//! uses it to decide the order it visits and updates its neighbors, which
//! replaces the default evaluator's position-dependent `HashSet` order.

use crate::pos::Dir;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SideBias {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Orientation {
    pub up: Dir,
    pub front: Dir,
    pub bias: SideBias,
}

fn cross(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl Orientation {
    pub fn new(up: Dir, front: Dir, bias: SideBias) -> Orientation {
        assert!(up.axis() != front.axis(), "up and front can not be on the same axis");
        Orientation { up, front, bias }
    }

    /// The side the orientation leans to: `up × front` (left of the front
    /// seen from above), flipped for a right bias.
    pub fn side(self) -> Dir {
        let c = cross(self.up.step(), self.front.step());
        let side = Dir::ALL.into_iter().find(|d| d.step() == c).expect("unit cross product");
        if self.bias == SideBias::Right { side.opposite() } else { side }
    }

    /// `getDirections`: the order neighbors are visited in.
    pub fn directions(self) -> [Dir; 6] {
        let side = self.side();
        [self.front.opposite(), self.front, side, side.opposite(), self.up.opposite(), self.up]
    }

    /// `getHorizontalDirections`: [`Self::directions`] off the up axis.
    pub fn horizontal(self) -> impl Iterator<Item = Dir> {
        let up = self.up.axis();
        self.directions().into_iter().filter(move |d| d.axis() != up)
    }

    /// `getVerticalDirections`: [`Self::directions`] on the up axis.
    pub fn vertical(self) -> impl Iterator<Item = Dir> {
        let up = self.up.axis();
        self.directions().into_iter().filter(move |d| d.axis() == up)
    }

    /// `withFront`: turning to face up or down tips `up` over.
    pub fn with_front(self, front: Dir) -> Orientation {
        let up = if front == self.up {
            self.front.opposite()
        } else if front == self.up.opposite() {
            self.front
        } else {
            self.up
        };
        Orientation::new(up, front, self.bias)
    }

    /// `withUp`: tipping `up` onto the front axis tips `front` over.
    pub fn with_up(self, up: Dir) -> Orientation {
        let front = if up == self.front {
            self.up.opposite()
        } else if up == self.front.opposite() {
            self.up
        } else {
            self.front
        };
        Orientation::new(up, front, self.bias)
    }

    /// `withFrontPreserveUp`: unchanged if `front` is on the up axis.
    pub fn with_front_preserve_up(self, front: Dir) -> Orientation {
        if front.axis() == self.up.axis() { self } else { self.with_front(front) }
    }

    pub fn with_bias(self, bias: SideBias) -> Orientation {
        Orientation { bias, ..self }
    }

    /// All 48 orientations, for picking one at random.
    pub fn all() -> impl Iterator<Item = Orientation> {
        Dir::ALL.into_iter().flat_map(|up| {
            Dir::ALL.into_iter().filter(move |f| f.axis() != up.axis()).flat_map(move |front| {
                [SideBias::Left, SideBias::Right].into_iter().map(move |bias| Orientation { up, front, bias })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_48_and_turning_keeps_axes_apart() {
        assert_eq!(Orientation::all().count(), 48);
        for o in Orientation::all() {
            for d in Dir::ALL {
                assert_ne!(o.with_front(d).up.axis(), d.axis());
                assert_ne!(o.with_up(d).front.axis(), d.axis());
            }
            let mut dirs = o.directions().to_vec();
            dirs.sort();
            assert_eq!(dirs, Dir::ALL.to_vec());
        }
    }
}
