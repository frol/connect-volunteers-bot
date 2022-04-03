use google_sheets4::api::ValueRange;
use google_sheets4::{hyper, hyper_rustls, oauth2, Sheets};
use teloxide::{
    dispatching2::dialogue::{serializer::Json, RedisStorage, Storage},
    macros::DialogueState,
    payloads::SendMessageSetters,
    prelude2::*,
    RequestError,
};
use thiserror::Error;

type MyDialogue = Dialogue<State, RedisStorage<Json>>;
type StorageError = <RedisStorage<Json> as Storage<State>>::Error;

#[derive(Debug, Error)]
enum Error {
    #[error("error from Telegram: {0}")]
    TelegramError(#[from] RequestError),

    #[error("error from storage: {0}")]
    StorageError(#[from] StorageError),
}

struct AppState {
    sheets_api: Sheets,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum HelpKind {
    ProvidingDriver,
    ProvidingUsefulContact,
    ProvidingCollectingHumanitarianHelp,
    NeedEvacuation,
    NeedHumanitarianHelp,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Contact {
    full_name: Option<String>,
    address: Option<String>,
    phone_numbers: Option<String>,
    comments: Option<String>,
}

#[derive(DialogueState, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[handler_out(anyhow::Result<()>)]
pub enum State {
    #[handler(handle_start)]
    Start,

    #[handler(handle_awaiting_kind_of_help_providing)]
    AwaitingKindOfHelpProviding,

    #[handler(handle_awaitig_kind_of_help_wanted)]
    AwaitingKindOfHelpWanted,

    #[handler(handle_awaiting_contact_information)]
    AwaitingContactInformation {
        help_kind: HelpKind,
        contact: Option<Contact>,
    },
}

impl Default for State {
    fn default() -> Self {
        Self::Start
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();
    log::info!("Starting bot...");

    // Get an ApplicationSecret instance by some means. It contains the `client_id` and
    // `client_secret`, among other things.
    let secret: oauth2::ApplicationSecret = serde_json::from_str(
        &std::env::var("COLLECT_VOLUNTEERS_BOT_OAUTH2_SECRET")
            .expect("Set COLLECT_VOLUNTEERS_BOT_OAUTH2_SECRET env variable"),
    )
    .unwrap();
    // Instantiate the authenticator. It will choose a suitable authentication flow for you,
    // unless you replace  `None` with the desired Flow.
    // Provide your own `AuthenticatorDelegate` to adjust the way it operates and get feedback about
    // what's going on. You probably want to bring in your own `TokenStorage` to persist tokens and
    // retrieve them from storage.
    let auth = oauth2::InstalledFlowAuthenticator::builder(
        secret,
        oauth2::InstalledFlowReturnMethod::HTTPRedirect,
    )
    .persist_tokens_to_disk(std::env::current_dir().unwrap().join("access_keys"))
    .build()
    .await
    .unwrap();
    let sheets_api = Sheets::new(
        hyper::Client::builder().build(hyper_rustls::HttpsConnector::with_native_roots()),
        auth,
    );

    let bot = Bot::from_env().auto_send();
    // You can also choose serializer::JSON or serializer::CBOR
    // All serializers but JSON require enabling feature
    // "serializer-<name>", e. g. "serializer-cbor"
    // or "serializer-bincode"
    let redis_url = std::env::var("COLLECT_VOLUNTEERS_BOT_REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let storage = RedisStorage::open(redis_url.as_str(), Json).await.unwrap();

    let app_state = AppState { sheets_api };

    let handler = Update::filter_message()
        .enter_dialogue::<Message, RedisStorage<Json>, State>()
        .dispatch_by::<State>();

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![std::sync::Arc::new(app_state), storage])
        .build()
        .setup_ctrlc_handler()
        .dispatch()
        .await;
}

fn start_keyboard() -> teloxide::types::KeyboardMarkup {
    teloxide::types::KeyboardMarkup::new(vec![vec![
        teloxide::types::KeyboardButton::new("Я можу допомогти"),
        teloxide::types::KeyboardButton::new("Я потребую допомоги"),
    ]])
}

async fn handle_start(
    bot: AutoSend<Bot>,
    msg: Message,
    dialogue: MyDialogue,
) -> anyhow::Result<()> {
    if !msg.chat.is_private() {
        log::info!("start: chat is not private: {:?}", msg.chat);
        return Ok(());
    }
    match msg.text() {
        Some("Я можу допомогти") => {
            dialogue.update(State::AwaitingKindOfHelpProviding).await?;
            bot.send_message(
                msg.chat.id,
                "Наразі в нас є можливість координувати водіїв, що допомогають з евакуацією, надавати гуманітарну допомогу, та ми завжди відкриті до корисних контактів. Оберіть один з варіантів.",
            ).reply_markup(teloxide::types::KeyboardMarkup::new(vec![vec![
                teloxide::types::KeyboardButton::new("Я водій з власним авто"),
                teloxide::types::KeyboardButton::new("Можу збирати гуманітарну чи фінансову допомогу"),
                teloxide::types::KeyboardButton::new("Корисні контакти"),
            ]]))
            .await?;
        }
        Some("Я потребую допомоги") => {
            dialogue.update(State::AwaitingKindOfHelpWanted).await?;
            bot.send_message(
                msg.chat.id,
                "Наразі ми координуємо запити на евакуацію та гуманітарну допомогу.",
            )
            .reply_markup(teloxide::types::KeyboardMarkup::new(vec![vec![
                teloxide::types::KeyboardButton::new("Евакуація"),
                teloxide::types::KeyboardButton::new("Потрібна гуманітарна допомога"),
            ]]))
            .await?;
        }
        _ => {
            log::info!("start: received unexpected type of message {:?}", msg);
            bot.send_message(
                msg.chat.id,
                "Оберіть \"Я можу допомогти\" чи \"Я потребую допомоги\"",
            )
            .reply_markup(start_keyboard())
            .await?;
        }
    }

    Ok(())
}

async fn handle_awaiting_kind_of_help_providing(
    bot: AutoSend<Bot>,
    msg: Message,
    dialogue: MyDialogue,
) -> anyhow::Result<()> {
    match msg.text() {
        Some("Я водій з власним авто") => {
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind: HelpKind::ProvidingDriver,
                    contact: None,
                })
                .await?;
        }
        Some("Корисні контакти") => {
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind: HelpKind::ProvidingUsefulContact,
                    contact: None,
                })
                .await?;
        }
        Some("Можу збирати гуманітарну чи фінансову допомогу") =>
        {
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind: HelpKind::ProvidingCollectingHumanitarianHelp,
                    contact: None,
                })
                .await?;
        }
        _ => {
            log::info!(
                "handle_awaitig_kind_of_help_wanted: received unexpected type of message {:?}",
                msg
            );
            bot.send_message(
                msg.chat.id,
                "Наразі в нас є можливість координувати водіїв, що допомогають з евакуацією, надавати гуманітарну допомогу, та ми завжди відкриті до корисних контактів. Оберіть один з варіантів.",
            ).reply_markup(teloxide::types::KeyboardMarkup::new(vec![vec![
                teloxide::types::KeyboardButton::new("Я водій з власним авто"),
                teloxide::types::KeyboardButton::new("Можу збирати гуманітарну чи фінансову допомогу"),
                teloxide::types::KeyboardButton::new("Корисні контакти"),
            ]]))
            .await?;
            return Ok(());
        }
    }

    bot.send_message(msg.chat.id, "Ваше ПІБ? (призвіще, імʼя, побатькові)")
        .reply_markup(teloxide::types::KeyboardRemove::new())
        .await?;

    Ok(())
}

async fn handle_awaitig_kind_of_help_wanted(
    bot: AutoSend<Bot>,
    msg: Message,
    dialogue: MyDialogue,
) -> anyhow::Result<()> {
    match msg.text() {
        Some("Евакуація") => {
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind: HelpKind::NeedEvacuation,
                    contact: None,
                })
                .await?;
        }
        Some("Потрібна гуманітарна допомога") => {
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind: HelpKind::NeedHumanitarianHelp,
                    contact: None,
                })
                .await?;
        }
        _ => {
            log::info!(
                "handle_awaitig_kind_of_help_wanted: received unexpected type of message {:?}",
                msg
            );
            bot.send_message(
                msg.chat.id,
                "Наразі ми координуємо запити на евакуацію та гуманітарну допомогу.",
            )
            .reply_markup(teloxide::types::KeyboardMarkup::new(vec![vec![
                teloxide::types::KeyboardButton::new("Евакуація"),
                teloxide::types::KeyboardButton::new("Потрібна гуманітарна допомога"),
            ]]))
            .await?;
            return Ok(());
        }
    }

    bot.send_message(msg.chat.id, "Ваше ПІБ? (призвіще, імʼя, побатькові)")
        .reply_markup(teloxide::types::KeyboardRemove::new())
        .await?;

    Ok(())
}

async fn handle_awaiting_contact_information(
    bot: AutoSend<Bot>,
    msg: Message,
    app_state: std::sync::Arc<AppState>,
    dialogue: MyDialogue,
    (help_kind, contact): (HelpKind, Option<Contact>),
) -> anyhow::Result<()> {
    let msg_text = if let Some(text) = msg.text() {
        text
    } else {
        return Ok(());
    };
    match contact {
        None => {
            let contact = Contact {
                full_name: Some(msg_text.to_owned()),
                ..Default::default()
            };
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind,
                    contact: Some(contact),
                })
                .await?;
            bot.send_message(msg.chat.id, "Контактні номери телефону?")
                .await?;
        }
        Some(
            mut contact @ Contact {
                phone_numbers: None,
                ..
            },
        ) => {
            contact.phone_numbers = Some(msg_text.to_owned());
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind,
                    contact: Some(contact),
                })
                .await?;
            bot.send_message(msg.chat.id, "Адреса?").await?;
        }
        Some(mut contact @ Contact { address: None, .. }) => {
            contact.address = Some(msg_text.to_owned());
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind,
                    contact: Some(contact),
                })
                .await?;
            bot.send_message(
                msg.chat.id,
                "Додатковий коментар? (якшо нема, відправте повідомлення з текстом \"-\")",
            )
            .await?;
        }
        Some(
            mut contact @ Contact {
                full_name: Some(_),
                phone_numbers: Some(_),
                address: Some(_),
                comments: None,
            },
        ) => {
            contact.comments = Some(msg_text.to_owned());
            let confirmation_msg = if let Contact {
                full_name: Some(full_name),
                phone_numbers: Some(phone_numbers),
                address: Some(address),
                comments: Some(comments),
            } = &contact
            {
                format!("Ось таку інформацію ми зібрали:\nПІБ: {full_name}\nКонтактні номери телефону: {phone_numbers}\nАдреса: {address}\nКоментар: {comments}\n\nВи бажаєте відправити цей запит волонтерам?")
            } else {
                log::warn!("Unexpected contact state: {:?}", contact);
                return Ok(());
            };
            dialogue
                .update(State::AwaitingContactInformation {
                    help_kind,
                    contact: Some(contact),
                })
                .await?;
            bot.send_message(msg.chat.id, confirmation_msg)
                .reply_markup(teloxide::types::KeyboardMarkup::new(vec![vec![
                    teloxide::types::KeyboardButton::new("Так, відправити інформацію волонтерам"),
                    teloxide::types::KeyboardButton::new("Ні, почати спочатку"),
                ]]))
                .await?;
        }
        Some(
            contact @ Contact {
                full_name: Some(_),
                phone_numbers: Some(_),
                address: Some(_),
                comments: Some(_),
            },
        ) => {
            let confirmed = match msg_text {
                "Так, відправити інформацію волонтерам" => true,
                "Ні, почати спочатку" => false,
                _ => {
                    bot.send_message(
                        msg.chat.id,
                        format!("Ви бажаєте відправити запит волонтерам? (відправте лише \"Так, відправити інформацію волонтерам\" або \"Ні, почати спочатку\""),
                    ).await?;
                    return Ok(());
                }
            };
            if confirmed {
                contact.save(&app_state.sheets_api, help_kind).await?;
            }
            dialogue.update(State::Start).await?;
            if confirmed {
                bot.send_message(
                    msg.chat.id,
                    "Дякуємо! Вашу інформацію відправлено волонтерам.\n\nЧекайте коли з вами звʼяжуться. Також можете надіслати іншу заявку.",
                ).reply_markup(start_keyboard())
                .await?;
            } else {
                bot.send_message(
                    msg.chat.id,
                    "Добре, вашу заявку скасовано. Можете почати знову.",
                )
                .reply_markup(start_keyboard())
                .await?;
            }
        }
        Some(contact) => {
            log::warn!("Unexpected contact state: {:?}", contact);
        }
    }

    Ok(())
}

impl Contact {
    async fn save(&self, sheets_api: &Sheets, help_kind: HelpKind) -> anyhow::Result<()> {
        let spreadsheet_id = match help_kind {
            HelpKind::ProvidingDriver => "117bcR8cksBSNUFNP51AAdr9pNMlJsFhwXL0NcbtW99A",
            HelpKind::ProvidingUsefulContact => "1K69NNDU2YnHnI9QSPO9FcUgjFZw70uPjncKNYTTWKHM",
            HelpKind::ProvidingCollectingHumanitarianHelp => {
                "1lfBO5dLNDW_ymL2aySJwtOqRAAttGaWp3QFPWYL5JlI"
            }
            HelpKind::NeedEvacuation => "1as4OGhZLULiQFqjgbHqnbed2xbiA4fCBjyYRbXPzHCU",
            HelpKind::NeedHumanitarianHelp => "1MM-8rxEcoD0GGqdTmudgchqpLIcaTygTN1x95nNzpJE",
        };
        // As the method needs a request, you would usually fill it with the desired information
        // into the respective structure. Some of the parts shown here might not be applicable !
        // Values shown here are possibly random and not representative !
        let values = if let Contact {
            full_name: Some(full_name),
            phone_numbers: Some(phone_numbers),
            address: Some(address),
            comments: Some(comments),
        } = self
        {
            Some(vec![vec![
                full_name.to_owned(),
                phone_numbers.to_owned(),
                address.to_owned(),
                comments.to_owned(),
                format!(
                    "{}",
                    chrono::Utc::now().with_timezone(&chrono::FixedOffset::east(3 * 3600))
                ),
            ]])
        } else {
            anyhow::bail!("Unexpected state of contact");
        };

        let req = ValueRange {
            major_dimension: Some("ROWS".to_owned()),
            range: None, //Some("A2:B2".to_owned()),
            values,
        };

        // You can configure optional parameters by calling the respective setters at will, and
        // execute the final call using `doit()`.
        // Values shown here are possibly random and not representative !
        let save_response = sheets_api
            .spreadsheets()
            .values_append(req, spreadsheet_id, "Sheet1")
            .value_input_option("USER_ENTERED")
            //.response_value_render_option("duo")
            //.response_date_time_render_option("ipsum")
            //.insert_data_option("gubergren")
            .include_values_in_response(true)
            .doit()
            .await?;

        log::info!("Saved: {:?}", save_response);

        Ok(())
    }
}
