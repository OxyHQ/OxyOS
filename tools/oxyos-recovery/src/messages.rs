//! What the recovery says, in English and Spanish, as the rest of OxyOS says it: plainly, in the
//! person's words, never a program's.

use oxyos_recovery::check::Outcome;
use oxyos_recovery::install::Step;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Spanish,
}

#[derive(Clone, Copy)]
pub struct Text {
    pub language: Language,
}

impl Text {
    pub fn new(language: Language) -> Self {
        Self { language }
    }

    fn pick(&self, english: &'static str, spanish: &'static str) -> &'static str {
        match self.language {
            Language::English => english,
            Language::Spanish => spanish,
        }
    }

    pub fn title(&self) -> &'static str {
        self.pick("OxyOS Recovery", "Recuperación de OxyOS")
    }

    pub fn could_not_start(&self) -> &'static str {
        self.pick(
            "OxyOS couldn't start several times in a row. You can restart and try again, check the disk, or erase and reinstall OxyOS.",
            "OxyOS no pudo iniciarse varias veces seguidas. Puedes reiniciar y volver a intentarlo, comprobar el disco o borrar y reinstalar OxyOS.",
        )
    }

    pub fn no_layout(&self) -> &'static str {
        self.pick(
            "This recovery can't find the disk OxyOS is installed on. Start the computer from OxyOS's installation medium to install it again.",
            "Esta recuperación no encuentra el disco en el que está instalado OxyOS. Inicia el ordenador desde el medio de instalación de OxyOS para volver a instalarlo.",
        )
    }

    pub fn converted(&self) -> &'static str {
        self.pick(
            "This computer was converted to OxyOS by hand. Its recovery can check and repair the disk, help with a forgotten password and get online; erasing it means installing OxyOS onto it from the installation medium.",
            "Este ordenador se pasó a OxyOS a mano. Su recuperación puede comprobar y reparar el disco, ayudarte si has olvidado la contraseña y conectarse a internet; para borrarlo hay que instalar OxyOS desde el medio de instalación.",
        )
    }

    pub fn converted_erase(&self) -> &'static str {
        self.pick(
            "This computer was converted to OxyOS by hand, and its disk isn't laid out as an installation: to erase it, start it from OxyOS's installation medium and install OxyOS. Nothing was changed.",
            "Este ordenador se pasó a OxyOS a mano y su disco no está organizado como una instalación: para borrarlo, inícialo desde el medio de instalación de OxyOS e instala OxyOS. No se ha cambiado nada.",
        )
    }

    pub fn menu(&self) -> [&'static str; 7] {
        match self.language {
            Language::English => [
                "Erase and Reinstall OxyOS",
                "Check and Repair the Disk",
                "Forgot Your Password?",
                "Wi-Fi",
                "Language: English",
                "Restart",
                "Shut Down",
            ],
            Language::Spanish => [
                "Borrar y reinstalar OxyOS",
                "Comprobar y reparar el disco",
                "¿Has olvidado tu contraseña?",
                "Wi-Fi",
                "Idioma: español",
                "Reiniciar",
                "Apagar",
            ],
        }
    }

    pub fn choose(&self) -> &'static str {
        self.pick("Choose an option: ", "Elige una opción: ")
    }

    pub fn press_enter(&self) -> &'static str {
        self.pick("Press Enter to go back. ", "Pulsa Intro para volver. ")
    }

    pub fn online(&self, online: bool) -> &'static str {
        match (self.language, online) {
            (Language::English, true) => "Connected to the internet.",
            (Language::English, false) => "Not connected to the internet.",
            (Language::Spanish, true) => "Conectado a internet.",
            (Language::Spanish, false) => "Sin conexión a internet.",
        }
    }

    pub fn erase_warning(&self) -> &'static str {
        self.pick(
            "This erases everything on this computer: every account's files, apps and settings, and this computer's settings. OxyOS is then installed again from this recovery, and the computer restarts into setup, as new.\nYour Oxy accounts themselves aren't deleted.",
            "Esto borra todo lo que hay en este ordenador: los archivos, las apps y los ajustes de cada cuenta, y los ajustes del ordenador. Después OxyOS se vuelve a instalar desde esta recuperación y el ordenador se reinicia en la configuración inicial, como nuevo.\nTus cuentas de Oxy no se eliminan.",
        )
    }

    pub fn confirm_word(&self) -> &'static str {
        self.pick("erase", "borrar")
    }

    pub fn type_to_confirm(&self) -> String {
        match self.language {
            Language::English => format!(
                "Type \"{}\" to erase this computer, or press Enter to cancel: ",
                self.confirm_word()
            ),
            Language::Spanish => format!(
                "Escribe \"{}\" para borrar este ordenador, o pulsa Intro para cancelar: ",
                self.confirm_word()
            ),
        }
    }

    pub fn cancelled(&self) -> &'static str {
        self.pick("Nothing was changed.", "No se ha cambiado nada.")
    }

    pub fn pending_erase(&self) -> &'static str {
        self.pick(
            "Erasing this computer, as was asked in Settings. Don't turn it off.",
            "Borrando este ordenador, como se pidió en Ajustes. No lo apagues.",
        )
    }

    pub fn step(&self, step: Step) -> &'static str {
        match step {
            Step::CheckingImage => self.pick(
                "Checking the installation image…",
                "Comprobando la imagen de instalación…",
            ),
            Step::Partitioning => self.pick(
                "Preparing the disk and copying OxyOS…",
                "Preparando el disco y copiando OxyOS…",
            ),
            Step::CheckingRecovery => {
                self.pick("Checking the recovery…", "Comprobando la recuperación…")
            }
            Step::SettingUp => self.pick("Setting OxyOS up…", "Configurando OxyOS…"),
            Step::Erasing => self.pick("Erasing the disk…", "Borrando el disco…"),
            Step::Recreating => self.pick("Installing OxyOS again…", "Volviendo a instalar OxyOS…"),
            Step::Finishing => self.pick("Finishing…", "Terminando…"),
        }
    }

    pub fn erased(&self) -> &'static str {
        self.pick(
            "This computer was erased. It restarts into setup.",
            "Este ordenador se ha borrado. Se reinicia en la configuración inicial.",
        )
    }

    pub fn erase_failed(&self) -> &'static str {
        self.pick(
            "Erasing didn't finish. The computer starts this recovery until it does: try again, or check the disk.",
            "El borrado no ha terminado. El ordenador inicia esta recuperación hasta que termine: vuelve a intentarlo o comprueba el disco.",
        )
    }

    pub fn ignored_request(&self) -> &'static str {
        self.pick(
            "An erase was asked for another installation of OxyOS, or can't be read. Nothing was erased.",
            "Se pidió borrar otra instalación de OxyOS, o la petición no se puede leer. No se ha borrado nada.",
        )
    }

    pub fn secret_prompt(&self) -> &'static str {
        self.pick(
            "The disk is encrypted. Enter the password of an account on this computer, or the recovery key: ",
            "El disco está cifrado. Introduce la contraseña de una cuenta de este ordenador o la clave de recuperación: ",
        )
    }

    pub fn wrong_secret(&self) -> &'static str {
        self.pick(
            "That password or recovery key doesn't unlock the disk.",
            "Esa contraseña o clave de recuperación no desbloquea el disco.",
        )
    }

    pub fn checking(&self) -> &'static str {
        self.pick("Checking the disk…", "Comprobando el disco…")
    }

    pub fn checked(&self, outcome: Outcome) -> &'static str {
        match outcome {
            Outcome::Clean => self.pick("No problems were found.", "No se han encontrado problemas."),
            Outcome::Repaired => self.pick("Problems were found and repaired.", "Se han encontrado y reparado problemas."),
            Outcome::Damaged => self.pick(
                "Problems were found that couldn't be repaired. Copy what you can, then erase and reinstall OxyOS.",
                "Se han encontrado problemas que no se pueden reparar. Copia lo que puedas y después borra y reinstala OxyOS.",
            ),
        }
    }

    pub fn esp_name(&self) -> &'static str {
        self.pick("Startup partition", "Partición de arranque")
    }

    pub fn system_name(&self) -> &'static str {
        self.pick("OxyOS and your files", "OxyOS y tus archivos")
    }

    pub fn forgot_password(&self) -> &'static str {
        self.pick(
            "Your password is your Oxy account's. To set a new one:\n\n  1. On another device, sign in at https://accounts.oxy.so/security with the code Oxy emails you, and set a new password there.\n  2. Restart this computer. When it asks for the password to unlock the disk, enter the recovery key you wrote down when you set it up.\n  3. Connect to the internet and sign in with your new password: this computer then takes it, for signing in and for unlocking the disk.\n\nNo one can get into your account or your files on this computer without your password or the recovery key, not even from here. If both are lost, you can only erase and reinstall OxyOS; your Oxy account and what it keeps online are not lost.",
            "Tu contraseña es la de tu cuenta de Oxy. Para poner una nueva:\n\n  1. En otro dispositivo, inicia sesión en https://accounts.oxy.so/security con el código que Oxy te envía por correo y pon allí una contraseña nueva.\n  2. Reinicia este ordenador. Cuando pida la contraseña para desbloquear el disco, introduce la clave de recuperación que anotaste al configurarlo.\n  3. Conéctate a internet e inicia sesión con tu nueva contraseña: desde entonces este ordenador la usa para iniciar sesión y para desbloquear el disco.\n\nNadie puede entrar en tu cuenta ni en tus archivos de este ordenador sin tu contraseña o la clave de recuperación, tampoco desde aquí. Si has perdido las dos, solo puedes borrar y reinstalar OxyOS; tu cuenta de Oxy y lo que guarda en línea no se pierden.",
        )
    }

    pub fn networks_heading(&self) -> &'static str {
        self.pick("Wi-Fi networks:", "Redes Wi-Fi:")
    }

    pub fn secured(&self) -> &'static str {
        self.pick(", password", ", con contraseña")
    }

    pub fn no_networks(&self) -> &'static str {
        self.pick(
            "No Wi-Fi networks were found.",
            "No se han encontrado redes Wi-Fi.",
        )
    }

    pub fn choose_network(&self) -> &'static str {
        self.pick(
            "Choose a network, or press Enter to go back: ",
            "Elige una red, o pulsa Intro para volver: ",
        )
    }

    pub fn network_password(&self) -> &'static str {
        self.pick("Password: ", "Contraseña: ")
    }

    pub fn joined(&self) -> &'static str {
        self.pick("Connected.", "Conectado.")
    }

    pub fn join_failed(&self) -> &'static str {
        self.pick(
            "Couldn't connect to that network.",
            "No se ha podido conectar a esa red.",
        )
    }

    pub fn restarting(&self) -> &'static str {
        self.pick("Restarting…", "Reiniciando…")
    }

    pub fn shutting_down(&self) -> &'static str {
        self.pick("Shutting down…", "Apagando…")
    }

    pub fn failed(&self) -> &'static str {
        self.pick("Something went wrong:", "Algo ha fallado:")
    }
}
