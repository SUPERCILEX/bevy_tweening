use std::cmp::min;
use std::time::Duration;

use bevy::prelude::*;

use crate::{EaseMethod, Lens, TweeningDirection, TweeningType};

/// Playback state of a [`Tweenable`].
///
/// This is returned by [`Tweenable::tick()`] to allow the caller to execute some logic based on the
/// updated state of the tweenable, like advanding a sequence to its next child tweenable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TweenState {
    /// The tweenable is still active, and did not reach its end state yet.
    Active,
    /// Animation reached its end state. The tweenable is idling at its latest time. This can only happen
    /// for [`TweeningType::Once`], since other types loop indefinitely.
    Completed,
}

/// Event raised when a tween completed.
///
/// This event is raised when a tween completed. For non-looping tweens, this is raised once at the
/// end of the animation. For looping animations, this is raised once per iteration. In case the animation
/// direction changes ([`TweeningType::PingPong`]), an iteration corresponds to a single progress from
/// one endpoint to the other, whatever the direction. Therefore a complete cycle start -> end -> start
/// counts as 2 iterations and raises 2 events (one when reaching the end, one when reaching back the start).
///
/// # Note
///
/// The semantic is slightly different from [`TweenState::Completed`], which indicates that the tweenable
/// has finished ticking and do not need to be updated anymore, a state which is never reached for looping
/// animation. Here the [`TweenCompleted`] event instead marks the end of a single loop iteration.
#[derive(Copy, Clone)]
pub struct TweenCompleted {
    /// The [`Entity`] the tween which completed and its animator are attached to.
    pub entity: Entity,
    /// An opaque value set by the user when activating event raising, used to identify the particular
    /// tween which raised this event. The value is passed unmodified from a call to [`with_completed_event()`]
    /// or [`set_completed_event()`].
    ///
    /// [`with_completed_event()`]: Tween::with_completed_event
    /// [`set_completed_event()`]: Tween::set_completed_event
    pub user_data: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct AnimClock {
    elapsed: Duration,
    duration: Duration,
    original: Duration,
    is_looping: bool,
}

impl AnimClock {
    fn new(duration: Duration, is_looping: bool) -> Self {
        AnimClock {
            elapsed: Duration::ZERO,
            duration,
            original: duration,
            is_looping,
        }
    }

    fn tick(&mut self, duration: Duration) -> u32 {
        self.elapsed += duration;

        if self.elapsed < self.duration {
            0
        } else if self.is_looping {
            let elapsed = self.elapsed.as_secs_f32();
            let duration = self.duration.as_secs_f32();

            self.elapsed = Duration::from_secs_f32(elapsed % duration);
            ((elapsed / duration) + 1e-5).floor() as u32
        } else {
            self.elapsed = self.duration;
            1
        }
    }

    fn set_progress(&mut self, progress: f32) {
        let progress = if self.is_looping {
            progress.fract()
        } else {
            progress.clamp(0., 1.)
        };

        self.elapsed = self.duration.mul_f32(progress);
    }

    fn progress(&self) -> f32 {
        self.elapsed.as_secs_f32() / self.duration.as_secs_f32()
    }

    fn completed(&self) -> bool {
        self.elapsed >= self.duration
    }

    fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }
}

/// An animatable entity, either a single [`Tween`] or a collection of them.
pub trait Tweenable<T>: Send + Sync {
    /// Get the total duration of the animation.
    ///
    /// For non-looping tweenables ([`TweeningType::Once`]), this is the total animation duration.
    /// For looping ones, this is the duration of a single iteration, since the total animation
    /// duration is infinite.
    ///
    /// Note that for [`TweeningType::PingPong`], this is the duration of a single way, either from
    /// start to end or back from end to start. The total "loop" duration start -> end -> start to
    /// reach back the same state in this case is the double of the returned value.
    fn duration(&self) -> Duration;

    /// Set the animation speed. Defaults to 1.
    ///
    /// Speeds greater than 1 slow down time. That is, a speed of 10 means the animation will
    /// take 10 times longer to complete whereas a speed of 0.5 means the animation will complete
    /// twice as fast.
    fn set_speed(&mut self, speed: f32);

    /// Return `true` if the animation is looping.
    ///
    /// Looping tweenables are of type [`TweeningType::Loop`] or [`TweeningType::PingPong`].
    fn is_looping(&self) -> bool;

    /// Set the current animation playback progress.
    ///
    /// See [`progress()`] for details on the meaning.
    ///
    /// [`progress()`]: Tweenable::progress
    fn set_progress(&mut self, progress: f32);

    /// Get the current progress in \[0:1\] (non-looping) or \[0:1\[ (looping) of the animation.
    ///
    /// For looping animations, this reports the progress of the current iteration, in the current
    /// direction:
    /// - [`TweeningType::Loop`] is `0` at start and `1` at end. The exact value `1.0` is never reached,
    ///   since the tweenable loops over to `0.0` immediately.
    /// - [`TweeningType::PingPong`] is `0` at the source endpoint and `1` and the destination one,
    ///   which are respectively the start/end for [`TweeningDirection::Forward`], or the end/start
    ///   for [`TweeningDirection::Backward`]. The exact value `1.0` is never reached, since the tweenable
    ///   loops over to `0.0` immediately when it changes direction at either endpoint.
    fn progress(&self) -> f32;

    /// Tick the animation, advancing it by the given delta time and mutating the given target component or asset.
    ///
    /// This returns [`TweenState::Active`] if the tweenable didn't reach its final state yet (progress < `1.0`),
    /// or [`TweenState::Completed`] if the tweenable completed this tick. Only non-looping tweenables return
    /// a completed state, since looping ones continue forever.
    ///
    /// Calling this method with a duration of [`Duration::ZERO`] is valid, and updates the target to the current
    /// state of the tweenable without actually modifying the tweenable state. This is useful after certain operations
    /// like [`rewind()`] or [`set_progress()`] whose effect is otherwise only visible on target on next frame.
    ///
    /// [`rewind()`]: Tweenable::rewind
    /// [`set_progress()`]: Tweenable::set_progress
    fn tick(
        &mut self,
        delta: Duration,
        target: &mut T,
        entity: Entity,
        event_writer: &mut EventWriter<TweenCompleted>,
    ) -> TweenState;

    /// Get the number of times this tweenable completed.
    ///
    /// For looping animations, this returns the number of times a single playback was completed. In the
    /// case of [`TweeningType::PingPong`] this corresponds to a playback in a single direction, so tweening
    /// from start to end and back to start counts as two completed times (one forward, one backward).
    fn times_completed(&self) -> u32;

    /// Rewind the animation to its starting state.
    ///
    /// Note that the starting state depends on the current direction. For [`TweeningDirection::Forward`]
    /// this is the start point of the lens, whereas for [`TweeningDirection::Backward`] this is the end one.
    fn rewind(&mut self);
}

impl<T> Tweenable<T> for Box<dyn Tweenable<T> + Send + Sync + 'static> {
    fn duration(&self) -> Duration {
        self.as_ref().duration()
    }
    fn set_speed(&mut self, speed: f32) {
        self.as_mut().set_speed(speed);
    }
    fn is_looping(&self) -> bool {
        self.as_ref().is_looping()
    }
    fn set_progress(&mut self, progress: f32) {
        self.as_mut().set_progress(progress);
    }
    fn progress(&self) -> f32 {
        self.as_ref().progress()
    }
    fn tick(
        &mut self,
        delta: Duration,
        target: &mut T,
        entity: Entity,
        event_writer: &mut EventWriter<TweenCompleted>,
    ) -> TweenState {
        self.as_mut().tick(delta, target, entity, event_writer)
    }
    fn times_completed(&self) -> u32 {
        self.as_ref().times_completed()
    }
    fn rewind(&mut self) {
        self.as_mut().rewind();
    }
}

/// Trait for boxing a [`Tweenable`] trait object.
pub trait IntoBoxDynTweenable<T> {
    /// Convert the current object into a boxed [`Tweenable`].
    fn into_box_dyn(this: Self) -> Box<dyn Tweenable<T> + Send + Sync + 'static>;
}

impl<T, U: Tweenable<T> + Send + Sync + 'static> IntoBoxDynTweenable<T> for U {
    fn into_box_dyn(this: U) -> Box<dyn Tweenable<T> + Send + Sync + 'static> {
        Box::new(this)
    }
}

/// Type of a callback invoked when a [`Tween`] has completed.
///
/// See [`Tween::set_completed()`] for usage.
pub type CompletedCallback<T> = dyn Fn(Entity, &Tween<T>) + Send + Sync + 'static;

/// Single tweening animation instance.
pub struct Tween<T> {
    ease_function: EaseMethod,
    clock: AnimClock,
    times_completed: u32,
    tweening_type: TweeningType,
    direction: TweeningDirection,
    lens: Box<dyn Lens<T> + Send + Sync + 'static>,
    on_completed: Option<Box<CompletedCallback<T>>>,
    event_data: Option<u64>,
}

impl<T: 'static> Tween<T> {
    /// Chain another [`Tweenable`] after this tween, making a [`Sequence`] with the two.
    ///
    /// # Example
    /// ```
    /// # use bevy_tweening::{lens::*, *};
    /// # use bevy::math::*;
    /// # use std::time::Duration;
    /// let tween1 = Tween::new(
    ///     EaseFunction::QuadraticInOut,
    ///     TweeningType::Once,
    ///     Duration::from_secs_f32(1.0),
    ///     TransformPositionLens {
    ///         start: Vec3::ZERO,
    ///         end: Vec3::new(3.5, 0., 0.),
    ///     },
    /// );
    /// let tween2 = Tween::new(
    ///     EaseFunction::QuadraticInOut,
    ///     TweeningType::Once,
    ///     Duration::from_secs_f32(1.0),
    ///     TransformRotationLens {
    ///         start: Quat::IDENTITY,
    ///         end: Quat::from_rotation_x(90.0_f32.to_radians()),
    ///     },
    /// );
    /// let seq = tween1.then(tween2);
    /// ```
    pub fn then(self, tween: impl Tweenable<T> + Send + Sync + 'static) -> Sequence<T> {
        Sequence::with_capacity(2).then(self).then(tween)
    }
}

impl<T> Tween<T> {
    /// Create a new tween animation.
    ///
    /// # Example
    /// ```
    /// # use bevy_tweening::{lens::*, *};
    /// # use bevy::math::Vec3;
    /// # use std::time::Duration;
    /// let tween = Tween::new(
    ///     EaseFunction::QuadraticInOut,
    ///     TweeningType::Once,
    ///     Duration::from_secs_f32(1.0),
    ///     TransformPositionLens {
    ///         start: Vec3::ZERO,
    ///         end: Vec3::new(3.5, 0., 0.),
    ///     },
    /// );
    /// ```
    pub fn new<L>(
        ease_function: impl Into<EaseMethod>,
        tweening_type: TweeningType,
        duration: Duration,
        lens: L,
    ) -> Self
    where
        L: Lens<T> + Send + Sync + 'static,
    {
        Tween {
            ease_function: ease_function.into(),
            clock: AnimClock::new(duration, tweening_type != TweeningType::Once),
            times_completed: 0,
            tweening_type,
            direction: TweeningDirection::Forward,
            lens: Box::new(lens),
            on_completed: None,
            event_data: None,
        }
    }

    /// Set the speed of the animation. See [Tweenable::set_speed] for details.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.set_speed(speed);
        self
    }

    /// Enable or disable raising a completed event.
    ///
    /// If enabled, the tween will raise a [`TweenCompleted`] event when the animation completed.
    /// This is similar to the [`set_completed()`] callback, but uses Bevy events instead.
    ///
    /// # Example
    /// ```
    /// # use bevy_tweening::{lens::*, *};
    /// # use bevy::{ecs::event::EventReader, math::Vec3};
    /// # use std::time::Duration;
    /// let tween = Tween::new(
    ///     // [...]
    /// #    EaseFunction::QuadraticInOut,
    /// #    TweeningType::Once,
    /// #    Duration::from_secs_f32(1.0),
    /// #    TransformPositionLens {
    /// #        start: Vec3::ZERO,
    /// #        end: Vec3::new(3.5, 0., 0.),
    /// #    },
    /// )
    /// .with_completed_event(true, 42);
    ///
    /// fn my_system(mut reader: EventReader<TweenCompleted>) {
    ///   for ev in reader.iter() {
    ///     assert_eq!(ev.user_data, 42);
    ///     println!("Entity {:?} raised TweenCompleted!", ev.entity);
    ///   }
    /// }
    /// ```
    ///
    /// [`set_completed()`]: Tween::set_completed
    pub fn with_completed_event(mut self, enabled: bool, user_data: u64) -> Self {
        self.event_data = if enabled { Some(user_data) } else { None };
        self
    }

    /// Set the playback direction of the tween.
    ///
    /// The playback direction influences the mapping of the progress ratio (in \[0:1\]) to the
    /// actual ratio passed to the lens. [`TweeningDirection::Forward`] maps the `0` value of
    /// progress to the `0` value of the lens ratio. Conversely, [`TweeningDirection::Backward`]
    /// reverses the mapping, which effectively makes the tween play reversed, going from end to
    /// start.
    ///
    /// Changing the direction doesn't change any target state, nor any progress of the tween. Only
    /// the direction of animation from this moment potentially changes. To force a target state
    /// change, call [`Tweenable::tick()`] with a zero delta (`Duration::ZERO`).
    pub fn set_direction(&mut self, direction: TweeningDirection) {
        self.direction = direction;
    }

    /// Set the playback direction of the tween.
    ///
    /// See [`Tween::set_direction()`].
    pub fn with_direction(mut self, direction: TweeningDirection) -> Self {
        self.direction = direction;
        self
    }

    /// The current animation direction.
    ///
    /// See [`TweeningDirection`] for details.
    pub fn direction(&self) -> TweeningDirection {
        self.direction
    }

    /// Set a callback invoked when the animation completed.
    ///
    /// The callback when invoked receives as parameters the [`Entity`] on which the target and the
    /// animator are, as well as a reference to the current [`Tween`].
    ///
    /// Only non-looping tweenables can complete.
    pub fn set_completed<C>(&mut self, callback: C)
    where
        C: Fn(Entity, &Tween<T>) + Send + Sync + 'static,
    {
        self.on_completed = Some(Box::new(callback));
    }

    /// Clear the callback invoked when the animation completed.
    pub fn clear_completed(&mut self) {
        self.on_completed = None;
    }

    /// Enable or disable raising a completed event.
    ///
    /// If enabled, the tween will raise a [`TweenCompleted`] event when the animation completed.
    /// This is similar to the [`set_completed()`] callback, but uses Bevy events instead.
    ///
    /// See [`with_completed_event()`] for details.
    ///
    /// [`set_completed()`]: Tween::set_completed
    /// [`with_completed_event()`]: Tween::with_completed_event
    pub fn set_completed_event(&mut self, enabled: bool, user_data: u64) {
        self.event_data = if enabled { Some(user_data) } else { None };
    }
}

impl<T> Tweenable<T> for Tween<T> {
    fn duration(&self) -> Duration {
        self.clock.duration
    }

    fn set_speed(&mut self, speed: f32) {
        let progress = self.progress();
        self.clock.duration = self.clock.original.mul_f32(speed);
        self.set_progress(progress);
    }

    fn is_looping(&self) -> bool {
        match self.tweening_type {
            TweeningType::Once => false,
            TweeningType::Loop | TweeningType::PingPong => true,
            TweeningType::LoopTimes(times) | TweeningType::PingPongTimes(times) => {
                self.times_completed < times
            }
        }
    }

    fn set_progress(&mut self, progress: f32) {
        self.clock.set_progress(progress);
    }

    fn progress(&self) -> f32 {
        self.clock.progress()
    }

    fn tick(
        &mut self,
        delta: Duration,
        target: &mut T,
        entity: Entity,
        event_writer: &mut EventWriter<TweenCompleted>,
    ) -> TweenState {
        if !self.is_looping() && self.clock.completed() {
            return TweenState::Completed;
        }

        // Tick the animation clock
        let times_completed = self.clock.tick(delta);
        self.times_completed += times_completed;
        if times_completed & 1 != 0
            && (self.tweening_type == TweeningType::PingPong
                || matches!(self.tweening_type, TweeningType::PingPongTimes(_)))
        {
            self.direction = !self.direction;
        }
        let state = if self.is_looping() || self.times_completed == 0 {
            TweenState::Active
        } else {
            TweenState::Completed
        };
        let progress = self.clock.progress();

        // Apply the lens, even if the animation finished, to ensure the state is consistent
        let mut factor = progress;
        if self.direction.is_backward() {
            factor = 1. - factor;
        }
        let factor = self.ease_function.sample(factor);
        self.lens.lerp(target, factor);

        // If completed at least once this frame, notify the user
        if times_completed > 0 {
            if let Some(user_data) = &self.event_data {
                event_writer.send(TweenCompleted {
                    entity,
                    user_data: *user_data,
                });
            }
            if let Some(cb) = &self.on_completed {
                cb(entity, self);
            }
        }

        state
    }

    fn times_completed(&self) -> u32 {
        self.times_completed
    }

    fn rewind(&mut self) {
        self.clock.reset();
        self.times_completed = 0;
    }
}

/// A sequence of tweens played back in order one after the other.
pub struct Sequence<T> {
    tweens: Vec<Box<dyn Tweenable<T> + Send + Sync + 'static>>,
    index: usize,
    duration: Duration,
    elapsed: Duration,
}

impl<T> Sequence<T> {
    /// Create a new sequence of tweens.
    ///
    /// This method panics if the input collection is empty.
    pub fn new(items: impl IntoIterator<Item = impl IntoBoxDynTweenable<T>>) -> Self {
        let tweens: Vec<_> = items
            .into_iter()
            .map(IntoBoxDynTweenable::into_box_dyn)
            .collect();
        assert!(!tweens.is_empty());
        let duration = tweens.iter().map(|t| t.duration()).sum();
        Sequence {
            tweens,
            index: 0,
            duration,
            elapsed: Duration::ZERO,
        }
    }

    /// Create a new sequence containing a single tween.
    pub fn from_single(tween: impl Tweenable<T> + Send + Sync + 'static) -> Self {
        let duration = tween.duration();
        Sequence {
            tweens: vec![Box::new(tween)],
            index: 0,
            duration,
            elapsed: Duration::ZERO,
        }
    }

    /// Create a new sequence with the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Sequence {
            tweens: Vec::with_capacity(capacity),
            index: 0,
            duration: Duration::ZERO,
            elapsed: Duration::ZERO,
        }
    }

    /// Append a [`Tweenable`] to this sequence.
    pub fn then(mut self, tween: impl Tweenable<T> + Send + Sync + 'static) -> Self {
        self.duration += tween.duration();
        self.tweens.push(Box::new(tween));
        self
    }

    /// Index of the current active tween in the sequence.
    pub fn index(&self) -> usize {
        self.index.min(self.tweens.len() - 1)
    }

    /// Get the current active tween in the sequence.
    pub fn current(&self) -> &dyn Tweenable<T> {
        self.tweens[self.index()].as_ref()
    }
}

impl<T> Tweenable<T> for Sequence<T> {
    fn duration(&self) -> Duration {
        self.duration
    }

    fn set_speed(&mut self, speed: f32) {
        for tween in &mut self.tweens {
            tween.set_speed(speed);
        }
    }

    fn is_looping(&self) -> bool {
        false // TODO - implement looping sequences...
    }

    fn set_progress(&mut self, progress: f32) {
        // Optimize the boundary conditions
        if progress < 1e-5 {
            self.rewind();
            return;
        } else if progress > 1. - 1e-5 {
            self.elapsed = self.duration;
            self.index = self.tweens.len();
            return;
        }

        self.elapsed = self.duration.mul_f32(progress.clamp(0., 1.));
        let mut delta = self.elapsed.as_secs_f32();

        // Use self.index to optimize out set_progress calls
        let mut index = 0;

        for tween in &mut self.tweens {
            let tween_duration = tween.duration().as_secs_f32();
            let tween_delta = tween_duration - delta;

            if tween_delta < -1e-5 {
                // Fully complete tween
                if index >= self.index {
                    tween.set_progress(1.);
                }
            } else {
                if tween_delta > 1e-5 {
                    // Partially complete tween
                    tween.set_progress(tween_delta / tween_duration);
                } else {
                    // We're right on the boundary of completing this tween, so mark it complete.
                    if index >= self.index {
                        tween.set_progress(1.);
                    }
                    index += 1;
                }

                break;
            }

            index += 1;
            delta -= tween_duration;
        }

        if index < self.index {
            let end = min(self.index + 1, self.tweens.len());
            for tween in &mut self.tweens[index + 1..end] {
                tween.rewind();
            }
        }
        self.index = index;
    }

    fn progress(&self) -> f32 {
        self.elapsed.as_secs_f32() / self.duration.as_secs_f32()
    }

    fn tick(
        &mut self,
        mut delta: Duration,
        target: &mut T,
        entity: Entity,
        event_writer: &mut EventWriter<TweenCompleted>,
    ) -> TweenState {
        self.elapsed = min(self.elapsed + delta, self.duration);

        let len = self.tweens.len();
        let mut state = TweenState::Completed;
        for tween in &mut self.tweens[self.index..] {
            let prev_progress = tween.progress();
            let prev_completions = tween.times_completed();

            state = tween.tick(delta, target, entity, event_writer);
            if state != TweenState::Completed {
                // If we completed zero times, then that means the entire delta was used up on this
                // tween. Otherwise, we need to diff the tween progress because it overlaps the
                // completion boundary.
                break;
            }
            self.index += 1;
            if self.index == len {
                // We've reached the end so we don't care about the remaining delta.
                break;
            }

            let tween_duration = tween.duration();

            let full_completions =
                (tween.times_completed() - prev_completions - 1) * tween_duration;
            delta -= full_completions;

            let used_delta = tween_duration.mul_f32(1. - prev_progress);
            if let Some(new_delta) = delta.checked_sub(used_delta) {
                delta = new_delta;
            } else {
                // We're some rounding error off of the finished tween, don't bother trying to
                // advance to the next one since delta would be zero.
                state = TweenState::Active;
                break;
            }
        }
        state
    }

    fn times_completed(&self) -> u32 {
        if self.index == self.tweens.len() {
            1
        } else {
            0
        }
    }

    fn rewind(&mut self) {
        self.elapsed = Duration::ZERO;
        self.index = 0;
        for tween in &mut self.tweens {
            tween.rewind();
        }
    }
}

/// A collection of [`Tweenable`] executing in parallel.
pub struct Tracks<T> {
    tracks: Vec<Box<dyn Tweenable<T> + Send + Sync + 'static>>,
    duration: Duration,
    elapsed: Duration,
    completed: bool,
}

impl<T> Tracks<T> {
    /// Create a new [`Tracks`] from an iterator over a collection of [`Tweenable`].
    pub fn new(items: impl IntoIterator<Item = impl IntoBoxDynTweenable<T>>) -> Self {
        let tracks: Vec<_> = items
            .into_iter()
            .map(IntoBoxDynTweenable::into_box_dyn)
            .collect();
        let duration = tracks.iter().map(|t| t.duration()).max().unwrap();
        Tracks {
            tracks,
            duration,
            elapsed: Duration::ZERO,
            completed: false,
        }
    }
}

impl<T> Tweenable<T> for Tracks<T> {
    fn duration(&self) -> Duration {
        self.duration
    }

    fn set_speed(&mut self, speed: f32) {
        for tween in &mut self.tracks {
            tween.set_speed(speed);
        }
    }

    fn is_looping(&self) -> bool {
        false // TODO - implement looping tracks...
    }

    fn set_progress(&mut self, progress: f32) {
        self.elapsed = self.duration.mul_f32(progress.clamp(0., 1.));
        let elapsed = self.elapsed.as_secs_f32();
        for tweenable in &mut self.tracks {
            tweenable.set_progress(elapsed / tweenable.duration().as_secs_f32());
        }
    }

    fn progress(&self) -> f32 {
        self.elapsed.as_secs_f32() / self.duration.as_secs_f32()
    }

    fn tick(
        &mut self,
        delta: Duration,
        target: &mut T,
        entity: Entity,
        event_writer: &mut EventWriter<TweenCompleted>,
    ) -> TweenState {
        self.elapsed = min(self.elapsed + delta, self.duration);

        let mut state = TweenState::Completed;
        for tweenable in &mut self.tracks {
            if tweenable.tick(delta, target, entity, event_writer) == TweenState::Active {
                state = TweenState::Active;
            }
        }
        self.completed = state == TweenState::Completed;
        state
    }

    fn times_completed(&self) -> u32 {
        if self.completed {
            1
        } else {
            0
        }
    }

    fn rewind(&mut self) {
        self.elapsed = Duration::ZERO;
        self.completed = false;
        for tween in &mut self.tracks {
            tween.rewind();
        }
    }
}

/// A time delay that doesn't animate anything.
///
/// This is generally useful for combining with other tweenables into sequences and tracks,
/// for example to delay the start of a tween in a track relative to another track. The `menu`
/// example (`examples/menu.rs`) uses this technique to delay the animation of its buttons.
pub struct Delay {
    timer: Timer,
    original: Duration,
}

impl Delay {
    /// Create a new [`Delay`] with a given duration.
    pub fn new(duration: Duration) -> Self {
        Delay {
            timer: Timer::new(duration, false),
            original: duration,
        }
    }

    /// Chain another [`Tweenable`] after this tween, making a sequence with the two.
    pub fn then<T>(self, tween: impl Tweenable<T> + Send + Sync + 'static) -> Sequence<T> {
        Sequence::with_capacity(2).then(self).then(tween)
    }
}

impl<T> Tweenable<T> for Delay {
    fn duration(&self) -> Duration {
        self.timer.duration()
    }

    fn set_speed(&mut self, speed: f32) {
        self.timer.set_duration(self.original.mul_f32(speed));
    }

    fn is_looping(&self) -> bool {
        false
    }

    fn set_progress(&mut self, progress: f32) {
        self.timer.reset();
        self.timer.tick(self.timer.duration().mul_f32(progress));
    }

    fn progress(&self) -> f32 {
        self.timer.percent()
    }

    fn tick(
        &mut self,
        delta: Duration,
        _target: &mut T,
        _entity: Entity,
        _event_writer: &mut EventWriter<TweenCompleted>,
    ) -> TweenState {
        self.timer.tick(delta);
        if self.timer.finished() {
            TweenState::Completed
        } else {
            TweenState::Active
        }
    }

    fn times_completed(&self) -> u32 {
        if self.timer.finished() {
            1
        } else {
            0
        }
    }

    fn rewind(&mut self) {
        self.timer.reset();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use bevy::ecs::{event::Events, system::SystemState};
    use itertools::Itertools;

    use crate::lens::*;

    use super::*;

    /// Utility to compare floating-point values with a tolerance.
    fn abs_diff_eq(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() < tol
    }

    #[derive(Default, Copy, Clone)]
    struct CallbackMonitor {
        invoke_count: u64,
        last_reported_count: u32,
    }

    /// Test ticking of a single tween in isolation.
    #[test]
    fn tween_tick() {
        for tweening_direction in &[TweeningDirection::Forward, TweeningDirection::Backward] {
            for tweening_type in &[
                TweeningType::Once,
                TweeningType::Loop,
                TweeningType::LoopTimes(1),
                TweeningType::PingPong,
                TweeningType::PingPongTimes(2),
            ] {
                println!(
                    "TweeningType: type={:?} dir={:?}",
                    tweening_type, tweening_direction
                );

                // Create a linear tween over 1 second
                let mut tween = Tween::new(
                    EaseMethod::Linear,
                    *tweening_type,
                    Duration::from_secs_f32(1.0),
                    TransformPositionLens {
                        start: Vec3::ZERO,
                        end: Vec3::ONE,
                    },
                )
                .with_direction(*tweening_direction);
                assert_eq!(tween.direction(), *tweening_direction);
                assert!(tween.on_completed.is_none());
                assert!(tween.event_data.is_none());

                let dummy_entity = Entity::from_raw(42);

                // Register callbacks to count started/ended events
                let callback_monitor = Arc::new(Mutex::new(CallbackMonitor::default()));
                let cb_mon_ptr = Arc::clone(&callback_monitor);
                tween.set_completed(move |entity, tween| {
                    assert_eq!(dummy_entity, entity);
                    let mut cb_mon = cb_mon_ptr.lock().unwrap();
                    cb_mon.invoke_count += 1;
                    cb_mon.last_reported_count = tween.times_completed();
                });
                assert!(tween.on_completed.is_some());
                assert!(tween.event_data.is_none());
                assert_eq!(callback_monitor.lock().unwrap().invoke_count, 0);

                // Activate event sending
                const USER_DATA: u64 = 54789; // dummy
                tween.set_completed_event(true, USER_DATA);
                assert!(tween.event_data.is_some());
                assert_eq!(tween.event_data.unwrap(), USER_DATA);

                // Dummy world and event writer
                let mut world = World::new();
                world.insert_resource(Events::<TweenCompleted>::default());
                let mut event_writer_system_state: SystemState<EventWriter<TweenCompleted>> =
                    SystemState::new(&mut world);
                let mut event_reader_system_state: SystemState<EventReader<TweenCompleted>> =
                    SystemState::new(&mut world);

                // Loop over 2.2 seconds, so greater than one ping-pong loop
                let mut transform = Transform::default();
                let tick_duration = Duration::from_secs_f32(0.2);
                for i in 1..=11 {
                    // Calculate expected values
                    let (progress, times_completed, mut direction, expected_state, just_completed) =
                        match tweening_type {
                            TweeningType::Once => {
                                let progress = (i as f32 * 0.2).min(1.0);
                                let times_completed = if i >= 5 { 1 } else { 0 };
                                let state = if i < 5 {
                                    TweenState::Active
                                } else {
                                    TweenState::Completed
                                };
                                let just_completed = i == 5;
                                (
                                    progress,
                                    times_completed,
                                    TweeningDirection::Forward,
                                    state,
                                    just_completed,
                                )
                            }
                            TweeningType::Loop | TweeningType::LoopTimes(_) => {
                                let progress = (i as f32 * 0.2).fract();
                                let times_completed = i / 5;
                                let just_completed = i % 5 == 0;
                                (
                                    progress,
                                    times_completed,
                                    TweeningDirection::Forward,
                                    if *tweening_type == TweeningType::Loop || i < 5 {
                                        TweenState::Active
                                    } else {
                                        TweenState::Completed
                                    },
                                    just_completed,
                                )
                            }
                            TweeningType::PingPong | TweeningType::PingPongTimes(_) => {
                                let i5 = i % 5;
                                let progress = i5 as f32 * 0.2;
                                let times_completed = i / 5;
                                let i10 = i % 10;
                                let direction = if i10 >= 5
                                    && (*tweening_type == TweeningType::PingPong || i < 10)
                                {
                                    TweeningDirection::Backward
                                } else {
                                    TweeningDirection::Forward
                                };
                                let just_completed = i5 == 0;
                                (
                                    progress,
                                    times_completed,
                                    direction,
                                    if *tweening_type == TweeningType::PingPong || i < 10 {
                                        TweenState::Active
                                    } else {
                                        TweenState::Completed
                                    },
                                    just_completed,
                                )
                            }
                        };
                    let factor = if tweening_direction.is_backward() {
                        direction = !direction;
                        1. - progress
                    } else {
                        progress
                    };
                    let expected_translation = if direction.is_forward() {
                        Vec3::splat(progress)
                    } else {
                        Vec3::splat(1. - progress)
                    };
                    println!(
                        "Expected: progress={} factor={} times_completed={} direction={:?} state={:?} just_completed={} translation={:?}",
                        progress, factor, times_completed, direction, expected_state, just_completed, expected_translation
                    );

                    // Tick the tween
                    let actual_state = {
                        let mut event_writer = event_writer_system_state.get_mut(&mut world);
                        tween.tick(
                            tick_duration,
                            &mut transform,
                            dummy_entity,
                            &mut event_writer,
                        )
                    };

                    // Propagate events
                    {
                        let mut events =
                            world.get_resource_mut::<Events<TweenCompleted>>().unwrap();
                        events.update();
                    }

                    // Check actual values
                    assert_eq!(tween.direction(), direction);
                    assert_eq!(
                        tween.is_looping(),
                        match *tweening_type {
                            TweeningType::Once => false,
                            TweeningType::Loop | TweeningType::PingPong => true,
                            TweeningType::LoopTimes(times) | TweeningType::PingPongTimes(times) => {
                                times_completed < times
                            }
                        }
                    );
                    assert_eq!(actual_state, expected_state);
                    assert!(abs_diff_eq(tween.progress(), progress, 1e-5));
                    assert_eq!(tween.times_completed(), times_completed);
                    assert!(transform
                        .translation
                        .abs_diff_eq(expected_translation, 1e-5));
                    assert!(transform.rotation.abs_diff_eq(Quat::IDENTITY, 1e-5));
                    let cb_mon = callback_monitor.lock().unwrap();
                    assert_eq!(cb_mon.invoke_count, times_completed as u64);
                    assert_eq!(cb_mon.last_reported_count, times_completed);
                    {
                        let mut event_reader = event_reader_system_state.get_mut(&mut world);
                        let event = event_reader.iter().next();
                        if just_completed {
                            assert!(event.is_some());
                            if let Some(event) = event {
                                assert_eq!(event.entity, dummy_entity);
                                assert_eq!(event.user_data, USER_DATA);
                            }
                        } else {
                            assert!(event.is_none());
                        }
                    }
                }

                // Rewind
                tween.rewind();
                assert_eq!(tween.direction(), *tweening_direction); // does not change
                assert_eq!(tween.is_looping(), *tweening_type != TweeningType::Once);
                assert!(abs_diff_eq(tween.progress(), 0., 1e-5));
                assert_eq!(tween.times_completed(), 0);

                // Dummy tick to update target
                let actual_state = {
                    let mut event_writer = event_writer_system_state.get_mut(&mut world);
                    tween.tick(
                        Duration::ZERO,
                        &mut transform,
                        Entity::from_raw(0),
                        &mut event_writer,
                    )
                };
                assert_eq!(actual_state, TweenState::Active);
                let expected_translation = if tweening_direction.is_backward() {
                    Vec3::ONE
                } else {
                    Vec3::ZERO
                };
                assert!(transform
                    .translation
                    .abs_diff_eq(expected_translation, 1e-5));
                assert!(transform.rotation.abs_diff_eq(Quat::IDENTITY, 1e-5));

                // Clear callback
                tween.clear_completed();
                assert!(tween.on_completed.is_none());
            }
        }
    }

    #[test]
    fn tween_dir() {
        let mut tween = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.0),
            TransformPositionLens {
                start: Vec3::ZERO,
                end: Vec3::ONE,
            },
        );

        // Default
        assert_eq!(tween.direction(), TweeningDirection::Forward);
        assert!(abs_diff_eq(tween.progress(), 0.0, 1e-5));

        // no-op
        tween.set_direction(TweeningDirection::Forward);
        assert_eq!(tween.direction(), TweeningDirection::Forward);
        assert!(abs_diff_eq(tween.progress(), 0.0, 1e-5));

        // Backward
        tween.set_direction(TweeningDirection::Backward);
        assert_eq!(tween.direction(), TweeningDirection::Backward);
        // progress is independent of direction
        assert!(abs_diff_eq(tween.progress(), 0.0, 1e-5));

        // Progress-invariant
        tween.set_direction(TweeningDirection::Forward);
        tween.set_progress(0.3);
        assert!(abs_diff_eq(tween.progress(), 0.3, 1e-5));
        tween.set_direction(TweeningDirection::Backward);
        // progress is independent of direction
        assert!(abs_diff_eq(tween.progress(), 0.3, 1e-5));

        // Dummy world and event writer
        let mut world = World::new();
        world.insert_resource(Events::<TweenCompleted>::default());
        let mut event_writer_system_state: SystemState<EventWriter<TweenCompleted>> =
            SystemState::new(&mut world);

        // Progress always increases alongside the current direction
        let dummy_entity = Entity::from_raw(0);
        let mut transform = Transform::default();
        let mut event_writer = event_writer_system_state.get_mut(&mut world);
        tween.set_direction(TweeningDirection::Backward);
        assert!(abs_diff_eq(tween.progress(), 0.3, 1e-5));
        tween.tick(
            Duration::from_secs_f32(0.1),
            &mut transform,
            dummy_entity,
            &mut event_writer,
        );
        assert!(abs_diff_eq(tween.progress(), 0.4, 1e-5));
        assert!(transform.translation.abs_diff_eq(Vec3::splat(0.6), 1e-5));
    }

    /// Test ticking a sequence of tweens.
    #[test]
    fn seq_tick() {
        let tween1 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.0),
            TransformPositionLens {
                start: Vec3::ZERO,
                end: Vec3::ONE,
            },
        );
        let tween2 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.0),
            TransformRotationLens {
                start: Quat::IDENTITY,
                end: Quat::from_rotation_x(90_f32.to_radians()),
            },
        );
        let mut seq = tween1.then(tween2);
        let mut transform = Transform::default();

        // Dummy world and event writer
        let mut world = World::new();
        world.insert_resource(Events::<TweenCompleted>::default());
        let mut system_state: SystemState<EventWriter<TweenCompleted>> =
            SystemState::new(&mut world);
        let mut event_writer = system_state.get_mut(&mut world);

        for i in 1..=16 {
            let state = seq.tick(
                Duration::from_secs_f32(0.2),
                &mut transform,
                Entity::from_raw(0),
                &mut event_writer,
            );
            if i < 5 {
                assert_eq!(state, TweenState::Active);
                let r = i as f32 * 0.2;
                assert_eq!(transform, Transform::from_translation(Vec3::splat(r)));
            } else if i < 10 {
                assert_eq!(state, TweenState::Active);
                let alpha_deg = (18 * (i - 5)) as f32;
                assert!(transform.translation.abs_diff_eq(Vec3::splat(1.), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(alpha_deg.to_radians()), 1e-5));
            } else {
                assert_eq!(state, TweenState::Completed);
                assert!(transform.translation.abs_diff_eq(Vec3::splat(1.), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(90_f32.to_radians()), 1e-5));
            }
        }
    }

    #[test]
    fn sequence_deltas_across_boundaries() {
        let tween1 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.0),
            TransformPositionLens {
                start: Vec3::ZERO,
                end: Vec3::ONE,
            },
        );
        let tween2 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.0),
            TransformRotationLens {
                start: Quat::IDENTITY,
                end: Quat::from_rotation_x(90_f32.to_radians()),
            },
        );
        let mut seq = tween1.then(tween2);
        let mut transform = Transform::default();

        // Dummy world and event writer
        let mut world = World::new();
        world.insert_resource(Events::<TweenCompleted>::default());
        let mut system_state: SystemState<EventWriter<TweenCompleted>> =
            SystemState::new(&mut world);
        let mut event_writer = system_state.get_mut(&mut world);

        for i in 1..=16 {
            let state = seq.tick(
                Duration::from_secs_f32(0.3),
                &mut transform,
                Entity::from_raw(0),
                &mut event_writer,
            );
            if i < 4 {
                assert_eq!(state, TweenState::Active);
                let r = i as f32 * 0.3;
                assert_eq!(transform, Transform::from_translation(Vec3::splat(r)));
            } else if i < 7 {
                assert_eq!(state, TweenState::Active);
                let alpha_deg = (18 + 27 * (i - 4)) as f32;
                assert!(transform.translation.abs_diff_eq(Vec3::splat(1.), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(alpha_deg.to_radians()), 1e-5));
            } else {
                assert_eq!(state, TweenState::Completed);
                assert!(transform.translation.abs_diff_eq(Vec3::splat(1.), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(90_f32.to_radians()), 1e-5));
            }
        }
    }

    #[test]
    fn sequence_delta_skips() {
        let tween1 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.0),
            TransformPositionLens {
                start: Vec3::ZERO,
                end: Vec3::ONE,
            },
        );
        let tween2 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.0),
            TransformRotationLens {
                start: Quat::IDENTITY,
                end: Quat::from_rotation_x(90_f32.to_radians()),
            },
        );
        let mut seq = tween1.then(tween2);
        let mut transform = Transform::default();

        // Dummy world and event writer
        let mut world = World::new();
        world.insert_resource(Events::<TweenCompleted>::default());
        let mut system_state: SystemState<EventWriter<TweenCompleted>> =
            SystemState::new(&mut world);
        let mut event_writer = system_state.get_mut(&mut world);

        for i in 1..=2 {
            let state = seq.tick(
                Duration::from_secs_f32(1.3),
                &mut transform,
                Entity::from_raw(0),
                &mut event_writer,
            );
            if i < 2 {
                assert_eq!(state, TweenState::Active);
                let alpha_deg = 27f32;
                assert!(transform.translation.abs_diff_eq(Vec3::splat(1.), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(alpha_deg.to_radians()), 1e-5));
            } else {
                assert_eq!(state, TweenState::Completed);
                assert!(transform.translation.abs_diff_eq(Vec3::splat(1.), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(90_f32.to_radians()), 1e-5));
            }
        }
    }

    /// Sequence::new() and various Sequence-specific methods
    #[test]
    fn seq_iter() {
        let mut seq = Sequence::new((1..5).map(|i| {
            Tween::new(
                EaseMethod::Linear,
                TweeningType::Once,
                Duration::from_secs_f32(0.2 * i as f32),
                TransformPositionLens {
                    start: Vec3::ZERO,
                    end: Vec3::ONE,
                },
            )
        }));
        assert!(!seq.is_looping());

        let mut progress = 0.;
        for i in 1..5 {
            assert_eq!(seq.index(), i - 1);
            assert!((seq.progress() - progress).abs() < 1e-5);
            let secs = 0.2 * i as f32;
            assert_eq!(seq.current().duration(), Duration::from_secs_f32(secs));
            progress += 0.25;
            seq.set_progress(progress);
            assert_eq!(seq.times_completed(), if i == 4 { 1 } else { 0 });
        }

        seq.rewind();
        assert_eq!(seq.progress(), 0.);
        assert_eq!(seq.times_completed(), 0);
    }

    #[test]
    fn sequence_set_progress_stress_tests() {
        let tweens = (0..5).map(|i| {
            Tween::new(
                EaseMethod::Linear,
                TweeningType::Once,
                Duration::from_secs_f32(0.2 * (1 << i) as f32),
                TransformPositionLens {
                    start: Vec3::ZERO,
                    end: Vec3::ONE,
                },
            )
        });
        let mut seq = Sequence::new(tweens.clone());
        let durations = tweens.map(|t| t.duration()).collect::<Vec<_>>();
        let total_time = durations.iter().sum::<Duration>().as_secs_f32();
        let progresses = durations
            .iter()
            .map(|d| d.as_secs_f32() / total_time)
            .collect::<Vec<_>>();
        let progression = (0..progresses.len())
            .map(|index| progresses[0..=index].iter().sum())
            .collect::<Vec<f32>>();

        for progress in [0., 0.1, 0.33, 0.5, 0.75, 0.95, 1., progression[3]]
            .iter()
            .permutations(8)
            .flatten()
        {
            seq.set_progress(*progress);
            assert!((seq.progress() - progress).abs() < 1e-5);

            assert_eq!(
                seq.index(),
                progression
                    .iter()
                    .find_position(|p| progress < p)
                    .map(|p| p.0)
                    .unwrap_or(progression.len() - 1)
            );
            assert_eq!(seq.current().duration(), durations[seq.index()]);
            assert_eq!(seq.times_completed(), if *progress == 1. { 1 } else { 0 });
        }
    }

    /// Test ticking parallel tracks of tweens.
    #[test]
    fn tracks_tick() {
        let tween1 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(1.),
            TransformPositionLens {
                start: Vec3::ZERO,
                end: Vec3::ONE,
            },
        );
        let tween2 = Tween::new(
            EaseMethod::Linear,
            TweeningType::Once,
            Duration::from_secs_f32(0.8), // shorter
            TransformRotationLens {
                start: Quat::IDENTITY,
                end: Quat::from_rotation_x(90_f32.to_radians()),
            },
        );
        let mut tracks = Tracks::new([tween1, tween2]);
        assert_eq!(tracks.duration(), Duration::from_secs_f32(1.)); // max(1., 0.8)
        assert!(!tracks.is_looping());

        let mut transform = Transform::default();

        // Dummy world and event writer
        let mut world = World::new();
        world.insert_resource(Events::<TweenCompleted>::default());
        let mut system_state: SystemState<EventWriter<TweenCompleted>> =
            SystemState::new(&mut world);
        let mut event_writer = system_state.get_mut(&mut world);

        for i in 1..=6 {
            let state = tracks.tick(
                Duration::from_secs_f32(0.2),
                &mut transform,
                Entity::from_raw(0),
                &mut event_writer,
            );
            if i < 5 {
                assert_eq!(state, TweenState::Active);
                assert_eq!(tracks.times_completed(), 0);
                let r = i as f32 * 0.2;
                assert!((tracks.progress() - r).abs() < 1e-5);
                let alpha_deg = 22.5 * i as f32;
                assert!(transform.translation.abs_diff_eq(Vec3::splat(r), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(alpha_deg.to_radians()), 1e-5));
            } else {
                assert_eq!(state, TweenState::Completed);
                assert_eq!(tracks.times_completed(), 1);
                assert!((tracks.progress() - 1.).abs() < 1e-5);
                assert!(transform.translation.abs_diff_eq(Vec3::splat(1.), 1e-5));
                assert!(transform
                    .rotation
                    .abs_diff_eq(Quat::from_rotation_x(90_f32.to_radians()), 1e-5));
            }
        }

        tracks.rewind();
        assert_eq!(tracks.times_completed(), 0);
        assert!(tracks.progress().abs() < 1e-5);

        tracks.set_progress(0.9);
        assert!((tracks.progress() - 0.9).abs() < 1e-5);
        // tick to udpate state (set_progress() does not update state)
        let state = tracks.tick(
            Duration::from_secs_f32(0.),
            &mut transform,
            Entity::from_raw(0),
            &mut event_writer,
        );
        assert_eq!(state, TweenState::Active);
        assert_eq!(tracks.times_completed(), 0);

        tracks.set_progress(3.2);
        assert!((tracks.progress() - 1.).abs() < 1e-5);
        // tick to udpate state (set_progress() does not update state)
        let state = tracks.tick(
            Duration::from_secs_f32(0.),
            &mut transform,
            Entity::from_raw(0),
            &mut event_writer,
        );
        assert_eq!(state, TweenState::Completed);
        assert_eq!(tracks.times_completed(), 1); // no looping

        tracks.set_progress(-0.5);
        assert!(tracks.progress().abs() < 1e-5);
        // tick to udpate state (set_progress() does not update state)
        let state = tracks.tick(
            Duration::from_secs_f32(0.),
            &mut transform,
            Entity::from_raw(0),
            &mut event_writer,
        );
        assert_eq!(state, TweenState::Active);
        assert_eq!(tracks.times_completed(), 0); // no looping
    }

    /// Test ticking a delay.
    #[test]
    fn delay_tick() {
        let duration = Duration::from_secs_f32(1.0);
        let mut delay = Delay::new(duration);
        {
            let tweenable: &dyn Tweenable<Transform> = &delay;
            assert_eq!(tweenable.duration(), duration);
            assert!(!tweenable.is_looping());
            assert!(tweenable.progress().abs() < 1e-5);
        }

        let mut transform = Transform::default();

        // Dummy world and event writer
        let mut world = World::new();
        world.insert_resource(Events::<TweenCompleted>::default());
        let mut system_state: SystemState<EventWriter<TweenCompleted>> =
            SystemState::new(&mut world);
        let mut event_writer = system_state.get_mut(&mut world);

        for i in 1..=6 {
            let state = delay.tick(
                Duration::from_secs_f32(0.2),
                &mut transform,
                Entity::from_raw(0),
                &mut event_writer,
            );
            {
                let tweenable: &dyn Tweenable<Transform> = &delay;
                if i < 5 {
                    assert_eq!(state, TweenState::Active);
                    let r = i as f32 * 0.2;
                    assert!((tweenable.progress() - r).abs() < 1e-5);
                } else {
                    assert_eq!(state, TweenState::Completed);
                    assert!((tweenable.progress() - 1.).abs() < 1e-5);
                }
            }
        }
    }
}
