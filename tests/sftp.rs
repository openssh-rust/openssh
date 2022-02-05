use openssh::*;

fn addr() -> String {
    std::env::var("TEST_HOST").unwrap_or("ssh://test-user@127.0.0.1:2222".to_string())
}

async fn connects() -> Vec<Session> {
    let mut sessions = Vec::with_capacity(2);

    #[cfg(feature = "process-mux")]
    {
        sessions.push(Session::connect(&addr(), KnownHosts::Accept).await.unwrap());
    }

    #[cfg(feature = "native-mux")]
    {
        sessions.push(
            Session::connect_mux(&addr(), KnownHosts::Accept)
                .await
                .unwrap(),
        );
    }

    sessions
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn it_connects() {
    for session in connects().await {
        session.check().await.unwrap();
        session.close().await.unwrap();
    }
}
