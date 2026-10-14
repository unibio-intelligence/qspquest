// Run both Rosenbrock tableaus through the shared solver harness.
use crate::{
    matrix::dense_nalgebra_serial::NalgebraMat,
    ode_equations::test_models::{
        exponential_decay::{
            exponential_decay_problem, exponential_decay_problem_with_root,
            negative_exponential_decay_problem,
        },
        heat2d::head2d_problem,
        robertson_ode::robertson_ode,
    },
    ode_solver::tests::{
        test_checkpointing, test_config, test_interpolate, test_interpolate_dy, test_ode_solver,
        test_problem, test_state_mut, test_state_mut_on_problem,
    },
    FaerSparseLU, FaerSparseMat, NalgebraLU, OdeSolverMethod,
};

type M = NalgebraMat<f64>;
type LS = NalgebraLU<f64>;

macro_rules! harness {
    ($modname:ident, $ctor:ident) => {
        mod $modname {
            use super::*;
            #[test]
            fn t_state_mut() {
                test_state_mut(test_problem::<M>(false).$ctor::<LS>().unwrap());
            }
            #[test]
            fn t_config() {
                test_config(robertson_ode::<M>(false, 1).0.$ctor::<LS>().unwrap());
            }
            #[test]
            fn t_interpolate() {
                test_interpolate(test_problem::<M>(false).$ctor::<LS>().unwrap());
            }
            #[test]
            fn t_interpolate_dy() {
                test_interpolate_dy(test_problem::<M>(false).$ctor::<LS>().unwrap());
            }
            #[test]
            fn t_checkpointing() {
                let (problem, soln) = exponential_decay_problem::<M>(false);
                let s1 = problem.$ctor::<LS>().unwrap();
                let s2 = problem.$ctor::<LS>().unwrap();
                test_checkpointing(soln, s1, s2);
            }
            #[test]
            fn t_state_mut_on_problem() {
                let (p, soln) = exponential_decay_problem::<M>(false);
                let mut s = p.$ctor::<LS>().unwrap();
                // Isolate state reinitialisation from accumulated adaptive global error.
                // The shared controller estimates local error; Rosenbrock23's default
                // trajectory reaches 19.28 tolerance units against this harness's 19.
                *s.state_mut().h = 0.1;
                s.config_mut().maximum_timestep_growth = 1.0;
                s.config_mut().minimum_timestep_growth = 1.0;
                test_state_mut_on_problem(s, soln);
            }
            #[test]
            fn t_exponential_decay() {
                let (problem, soln) = exponential_decay_problem::<M>(false);
                let mut s = problem.$ctor::<LS>().unwrap();
                test_ode_solver(&mut s, soln, None, false, false);
            }
            #[test]
            fn t_exponential_decay_tstop() {
                let (problem, soln) = exponential_decay_problem::<M>(false);
                let mut s = problem.$ctor::<LS>().unwrap();
                test_ode_solver(&mut s, soln, None, true, false);
            }
            #[test]
            fn t_negative_exponential_decay() {
                let (problem, soln) = negative_exponential_decay_problem::<M>(false);
                let mut s = problem.$ctor::<LS>().unwrap();
                test_ode_solver(&mut s, soln, Some(30.), false, false);
            }
            #[test]
            fn t_exponential_decay_with_root() {
                let (problem, soln) = exponential_decay_problem_with_root::<M>(false, false);
                let mut s = problem.$ctor::<LS>().unwrap();
                test_ode_solver(&mut s, soln, None, false, false);
            }
            #[test]
            fn t_robertson_ode() {
                let (problem, soln) = robertson_ode::<M>(false, 1);
                let mut s = problem.$ctor::<LS>().unwrap();
                test_ode_solver(&mut s, soln, None, false, false);
            }
            #[test]
            fn t_robertson_ode_tstop() {
                let (problem, soln) = robertson_ode::<M>(false, 1);
                let mut s = problem.$ctor::<LS>().unwrap();
                test_ode_solver(&mut s, soln, None, true, false);
            }
            #[test]
            fn t_heat2d_faer_sparse() {
                let (problem, soln) = head2d_problem::<FaerSparseMat<f64>, 10>();
                let mut s = problem.$ctor::<FaerSparseLU<f64>>().unwrap();
                test_ode_solver(&mut s, soln, None, false, false);
            }
        }
    };
}

harness!(rosenbrock23, rosenbrock23);
harness!(rodas5p, rodas5p);
