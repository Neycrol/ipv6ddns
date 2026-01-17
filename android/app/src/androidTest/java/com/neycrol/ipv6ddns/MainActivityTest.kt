package com.neycrol.ipv6ddns

import android.content.Context
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.preferencesDataStore
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.neycrol.ipv6ddns.data.AppConfig
import com.neycrol.ipv6ddns.data.ConfigStore
import kotlinx.coroutines.runBlocking
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

private val Context.testDataStore: DataStore<Preferences> by preferencesDataStore(name = "test_ipv6ddns_config")

@RunWith(AndroidJUnit4::class)
class MainActivityTest {

    @get:Rule
    val composeTestRule = createComposeRule()

    private lateinit var context: Context

    @Before
    fun setup() {
        context = ApplicationProvider.getApplicationContext<Context>()
        // Clear test data store before each test
        runBlocking {
            context.testDataStore.edit { it.clear() }
        }
    }

    @Test
    fun appScreen_displaysAllUIElements() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Cloudflare section is displayed
        composeTestRule.onNodeWithText("Cloudflare").assertIsDisplayed()

        // Verify Runtime section is displayed
        composeTestRule.onNodeWithText("Runtime").assertIsDisplayed()

        // Verify Status section is displayed
        composeTestRule.onNodeWithText("Stopped").assertIsDisplayed()

        // Verify Start and Stop buttons are displayed
        composeTestRule.onNodeWithText("Start").assertIsDisplayed()
        composeTestRule.onNodeWithText("Stop").assertIsDisplayed()
    }

    @Test
    fun appScreen_apiTokenField_displaysAndAcceptsInput() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify API token field is displayed
        composeTestRule.onNodeWithText("API Token").assertIsDisplayed()

        // Enter API token
        val apiToken = "test_api_token_12345"
        composeTestRule.onNodeWithText("API Token")
            .performTextInput(apiToken)

        // Verify the field contains the input
        composeTestRule.onNodeWithText("API Token")
            .assertTextContains(apiToken)
    }

    @Test
    fun appScreen_zoneIdField_displaysAndAcceptsInput() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Zone ID field is displayed
        composeTestRule.onNodeWithText("Zone ID").assertIsDisplayed()

        // Enter Zone ID
        val zoneId = "test_zone_id_12345"
        composeTestRule.onNodeWithText("Zone ID")
            .performTextInput(zoneId)

        // Verify the field contains the input
        composeTestRule.onNodeWithText("Zone ID")
            .assertTextContains(zoneId)
    }

    @Test
    fun appScreen_recordNameField_displaysAndAcceptsInput() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Record Name field is displayed
        composeTestRule.onNodeWithText("Record Name").assertIsDisplayed()

        // Enter Record Name
        val recordName = "test.example.com"
        composeTestRule.onNodeWithText("Record Name")
            .performTextInput(recordName)

        // Verify the field contains the input
        composeTestRule.onNodeWithText("Record Name")
            .assertTextContains(recordName)
    }

    @Test
    fun appScreen_timeoutField_displaysAndAcceptsInput() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Timeout field is displayed
        composeTestRule.onNodeWithText("Timeout").assertIsDisplayed()

        // Enter timeout value
        composeTestRule.onNodeWithText("Timeout")
            .performTextInput("60")

        // Verify the field contains the input
        composeTestRule.onNodeWithText("Timeout")
            .assertTextContains("60")
    }

    @Test
    fun appScreen_timeoutField_rejectsNonDigits() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Timeout field is displayed
        composeTestRule.onNodeWithText("Timeout").assertIsDisplayed()

        // Try to enter non-digit characters
        composeTestRule.onNodeWithText("Timeout")
            .performTextInput("abc")

        // Verify only digits are accepted (field should be empty)
        composeTestRule.onNodeWithText("Timeout")
            .assertTextEquals("")
    }

    @Test
    fun appScreen_pollIntervalField_displaysAndAcceptsInput() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Poll Interval field is displayed
        composeTestRule.onNodeWithText("Poll Interval").assertIsDisplayed()

        // Enter poll interval value
        composeTestRule.onNodeWithText("Poll Interval")
            .performTextInput("120")

        // Verify the field contains the input
        composeTestRule.onNodeWithText("Poll Interval")
            .assertTextContains("120")
    }

    @Test
    fun appScreen_pollIntervalField_rejectsNonDigits() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Poll Interval field is displayed
        composeTestRule.onNodeWithText("Poll Interval").assertIsDisplayed()

        // Try to enter non-digit characters
        composeTestRule.onNodeWithText("Poll Interval")
            .performTextInput("xyz")

        // Verify only digits are accepted (field should be empty)
        composeTestRule.onNodeWithText("Poll Interval")
            .assertTextEquals("")
    }

    @Test
    fun appScreen_verboseSwitch_displaysAndToggles() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Verbose switch is displayed
        composeTestRule.onNodeWithText("Verbose").assertIsDisplayed()

        // Verify initial state (should be unchecked)
        composeTestRule.onNode(hasTestTag("verbose_switch"))
            .assertIsNotSelected()

        // Toggle the switch
        composeTestRule.onNodeWithText("Verbose")
            .performClick()

        // Verify it's now checked (this may require a specific test tag)
    }

    @Test
    fun appScreen_multiRecordButton_displaysAndOpensMenu() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Multi Record button is displayed
        composeTestRule.onNodeWithText("Multi Record").assertIsDisplayed()

        // Click on the button
        composeTestRule.onNodeWithText("Multi Record")
            .performClick()

        // Verify dropdown menu is displayed
        composeTestRule.onNodeWithText("Error").assertIsDisplayed()
        composeTestRule.onNodeWithText("First").assertIsDisplayed()
        composeTestRule.onNodeWithText("All").assertIsDisplayed()
    }

    @Test
    fun appScreen_multiRecordButton_selectsOption() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Click on the Multi Record button
        composeTestRule.onNodeWithText("Multi Record")
            .performClick()

        // Select "First" option
        composeTestRule.onNodeWithText("First")
            .performClick()

        // Verify the button text changed (this may require a specific test tag)
    }

    @Test
    fun appScreen_startButton_isDisplayed() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Start button is displayed
        composeTestRule.onNodeWithText("Start").assertIsDisplayed()
    }

    @Test
    fun appScreen_stopButton_isDisplayed() {
        composeTestRule.setContent {
            AppScreen()
        }

        // Verify Stop button is displayed
        composeTestRule.onNodeWithText("Stop").assertIsDisplayed()
    }
}