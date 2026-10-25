// @generated

pub mod pins {
    ariel_os_hal::define_peripherals!(LedPeripherals { led0 : P0_00, led1 : P0_01, });
    ariel_os_hal::define_peripherals!(
        ButtonPeripherals { button0 : P0_02, button1 : P0_03, }
    );
    ariel_os_hal::define_i2c_buses![
        { name : I2c0, peripheral : I2C0, sda : P0_06, scl : P0_07, aliases : [I2C0,] },
    ];
    ariel_os_hal::define_uarts![
        { name : Uart0, device : UART0, tx : P0_05, rx : P0_04, host_facing : true },
    ];
}
#[allow(unused_variables)]
pub fn init(peripherals: &mut ariel_os_hal::hal::OptionalPeripherals) {}
